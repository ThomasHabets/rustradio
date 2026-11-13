//! Device configuration and lifetime management shared by audio blocks.
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SupportedStreamConfig, SupportedStreamConfigRange};

use crate::{Error, Result};

crate::error_from!("audio", cpal::Error);

fn select_config(
    configs: impl Iterator<Item = SupportedStreamConfigRange>,
    rate: u32,
) -> Result<SupportedStreamConfig> {
    configs
        .filter(|c| {
            c.channels() > 0
                && c.contains_rate(rate)
                && matches!(
                    c.sample_format(),
                    SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
                )
        })
        .min_by_key(|c| {
            (
                c.channels() != 1,
                c.sample_format() != SampleFormat::F32,
                c.channels(),
            )
        })
        .map(|c| c.with_sample_rate(rate))
        .ok_or_else(|| {
            Error::msg(format!(
                "audio device has no supported configuration at {rate} Hz"
            ))
        })
}

pub(crate) fn device_config(
    rate: u64,
    name: Option<&str>,
    input: bool,
) -> Result<(Device, SupportedStreamConfig)> {
    let rate = u32::try_from(rate)
        .ok()
        .filter(|&r| r != 0)
        .ok_or_else(|| Error::msg("audio sample rate must be nonzero and fit u32"))?;
    // A host-prefixed ID selects its backend explicitly; plain strings match
    // descriptions on the default backend. Never fall back for a missing ID.
    let id = name
        .filter(|name| name.contains(':'))
        .map(str::parse::<cpal::DeviceId>)
        .transpose()?;
    let host = match &id {
        Some(id) => cpal::host_from_id(id.host())?,
        None => cpal::default_host(),
    };
    let direction = if input { "input" } else { "output" };
    let device = if let Some(id) = id {
        host.device_by_id(&id)
            .filter(|d| {
                if input {
                    d.supports_input()
                } else {
                    d.supports_output()
                }
            })
            .ok_or_else(|| {
                Error::msg(format!(
                    "audio {direction} device {id} not found or does not support {direction}"
                ))
            })?
    } else if let Some(name) = name {
        let query_error = |e| {
            Error::device(
                e,
                format!("enumerating audio {direction} devices on {}", host.id()),
            )
        };
        let mut devices = if input {
            host.input_devices().map_err(query_error)?
        } else {
            host.output_devices().map_err(query_error)?
        };
        devices
            .find(|d| d.description().is_ok_and(|d| d.name().contains(name)))
            .ok_or_else(|| {
                Error::msg(format!(
                    "audio {direction} device matching {name:?} not found"
                ))
            })?
    } else if input {
        host.default_input_device()
            .ok_or_else(|| Error::msg("no default audio input device"))?
    } else {
        host.default_output_device()
            .ok_or_else(|| Error::msg("no default audio output device"))?
    };
    let description = device.description()?;
    let id = device.id()?;
    let context = format!("audio {direction} device {description} ({id}) at {rate} Hz");
    let query_error = |e| Error::device(e, format!("querying {context}"));
    let config = if input {
        select_config(device.supported_input_configs().map_err(query_error)?, rate)
    } else {
        select_config(
            device.supported_output_configs().map_err(query_error)?,
            rate,
        )
    }
    .map_err(|e| Error::wrap(e, format!("configuring {context}")))?;
    log::info!("Audio {direction} device: {description} ({id}), {config:?}");
    Ok((device, config))
}

/// Owns a paused CPAL stream and forwards callback errors to the graph worker.
pub(crate) struct AudioStream {
    // CPAL backends are not unwind-safe. Mutex preserves the blocks' unwind
    // traits; exclusive worker access uses get_mut(), without taking a lock.
    stream: Mutex<cpal::Stream>,
    // Access uses &mut self; Mutex supplies Sync without locking when polled.
    errors: Mutex<Receiver<cpal::Error>>,
    playing: bool,
}

impl AudioStream {
    pub(crate) fn new(
        start: impl FnOnce(Sender<cpal::Error>) -> Result<cpal::Stream>,
    ) -> Result<Self> {
        let (error_tx, errors) = channel();
        Ok(Self {
            stream: Mutex::new(start(error_tx)?),
            errors: Mutex::new(errors),
            playing: false,
        })
    }

    pub(crate) fn play(&mut self) -> Result<()> {
        if !self.playing {
            self.stream.get_mut().unwrap().play()?;
            self.playing = true;
        }
        Ok(())
    }

    pub(crate) fn now(&mut self) -> cpal::StreamInstant {
        self.stream.get_mut().unwrap().now()
    }

    pub(crate) fn check_error(&mut self) -> Result<()> {
        check_stream_errors(self.errors.get_mut().unwrap())
    }
}

// CPAL delivers recoverable backend notifications through the same callback as
// fatal device errors. Drain notifications so a later fatal error is not hidden.
fn check_stream_errors(errors: &Receiver<cpal::Error>) -> Result<()> {
    for error in errors.try_iter() {
        match error.kind() {
            cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied => {
                log::warn!("Audio stream notification: {error}");
            }
            _ => return Err(Error::device(error, "audio stream")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_rate_channels_and_format() -> Result<()> {
        let configs = [
            SupportedStreamConfigRange::new(
                2,
                44100,
                48000,
                cpal::SupportedBufferSize::Unknown,
                SampleFormat::F32,
            ),
            SupportedStreamConfigRange::new(
                1,
                48000,
                48000,
                cpal::SupportedBufferSize::Unknown,
                SampleFormat::I16,
            ),
        ];
        assert_eq!(select_config(configs.into_iter(), 48000)?.channels(), 1);
        assert_eq!(select_config(configs.into_iter(), 44100)?.channels(), 2);
        assert!(select_config(configs.into_iter(), 96000).is_err());
        assert!(select_config(configs.into_iter(), 0).is_err());
        Ok(())
    }

    #[test]
    fn invalid_device_ids_are_rejected_without_using_the_default_device() {
        for input in [false, true] {
            let error = device_config(48000, Some("unknown-backend:device"), input)
                .expect_err("invalid device ID must fail");
            assert!(error.to_string().contains("unknown-backend"));
        }
    }

    #[test]
    fn recoverable_notifications_do_not_hide_device_errors() {
        use cpal::ErrorKind;

        let (sender, errors) = channel();
        for kind in [ErrorKind::Xrun, ErrorKind::RealtimeDenied] {
            sender.send(cpal::Error::new(kind)).unwrap();
        }
        assert!(check_stream_errors(&errors).is_ok());
        sender.send(cpal::Error::new(ErrorKind::Xrun)).unwrap();
        sender
            .send(cpal::Error::new(ErrorKind::DeviceNotAvailable))
            .unwrap();
        let Error::DeviceError { source, .. } = check_stream_errors(&errors).unwrap_err() else {
            panic!("device error must preserve the CPAL error");
        };
        assert_eq!(
            source.downcast_ref::<cpal::Error>().unwrap().kind(),
            ErrorKind::DeviceNotAvailable
        );
    }

    #[test]
    fn startup_errors_are_returned() {
        assert!(AudioStream::new(|_| Err(Error::msg("test failure"))).is_err());
    }
}
