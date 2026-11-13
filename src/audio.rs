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
    let host = cpal::default_host();
    let direction = if input { "input" } else { "output" };
    let device = if let Some(name) = name {
        let mut devices = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
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
    let config = if input {
        select_config(device.supported_input_configs()?, rate)?
    } else {
        select_config(device.supported_output_configs()?, rate)?
    };
    log::info!(
        "Audio {direction} device: {}, {config:?}",
        device.description()?
    );
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

    pub(crate) fn check_error(&mut self) -> Result<()> {
        if let Ok(e) = self.errors.get_mut().unwrap().try_recv() {
            return Err(Error::msg(format!("audio stream error: {e}")));
        }
        Ok(())
    }
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
    fn startup_errors_are_returned() {
        assert!(AudioStream::new(|_| Err(Error::msg("test failure"))).is_err());
    }
}
