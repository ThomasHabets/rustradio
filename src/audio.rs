//! Device configuration and lifetime management shared by audio blocks.
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, SupportedStreamConfig, SupportedStreamConfigRange};

use crate::{Error, Result};

crate::error_from!(
    "audio",
    cpal::PlayStreamError,
    cpal::BuildStreamError,
    cpal::DevicesError,
    cpal::DeviceNameError,
    cpal::SupportedStreamConfigsError,
    cpal::DefaultStreamConfigError,
);

fn select_config(
    configs: impl Iterator<Item = SupportedStreamConfigRange>,
    rate: u32,
) -> Result<SupportedStreamConfig> {
    configs
        .filter(|c| {
            c.channels() > 0
                && c.min_sample_rate().0 <= rate
                && rate <= c.max_sample_rate().0
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
        .map(|c| c.with_sample_rate(SampleRate(rate)))
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
            .find(|d| d.name().is_ok_and(|n| n.contains(name)))
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
    log::info!("Audio {direction} device: {}, {config:?}", device.name()?);
    Ok((device, config))
}

/// CPAL streams stay on their owning thread because they need not be `Send`.
pub(crate) struct AudioStream {
    commands: Option<Sender<Command>>,
    thread: Option<std::thread::JoinHandle<()>>,
    // Access uses &mut self; Mutex supplies Sync without locking when polled.
    errors: Mutex<Receiver<cpal::StreamError>>,
    playing: bool,
}

enum Command {
    Play(Sender<Result<()>>),
}

impl AudioStream {
    pub(crate) fn new(
        name: &str,
        start: impl FnOnce(Sender<cpal::StreamError>) -> Result<cpal::Stream> + Send + 'static,
    ) -> Result<Self> {
        let (ready_tx, ready_rx) = channel();
        let (commands, requests) = channel();
        let (error_tx, errors) = channel();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || match start(error_tx) {
                Ok(stream) => {
                    if ready_tx.send(Ok(())).is_ok() {
                        while let Ok(Command::Play(reply)) = requests.recv() {
                            let _ = reply.send(stream.play().map_err(Into::into));
                        }
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        let owner = Self {
            commands: Some(commands),
            thread: Some(thread),
            errors: Mutex::new(errors),
            playing: false,
        };
        ready_rx.recv()??;
        Ok(owner)
    }

    pub(crate) fn play(&mut self) -> Result<()> {
        if !self.playing {
            let (tx, rx) = channel();
            self.commands
                .as_ref()
                .unwrap()
                .send(Command::Play(tx))
                .map_err(|_| Error::msg("audio stream thread stopped"))?;
            rx.recv()??;
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

impl Drop for AudioStream {
    fn drop(&mut self) {
        self.commands.take();
        if let Some(thread) = self.thread.take()
            && let Err(e) = thread.join()
        {
            log::error!("Audio stream thread panicked: {e:?}");
        }
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
                SampleRate(44100),
                SampleRate(48000),
                cpal::SupportedBufferSize::Unknown,
                SampleFormat::F32,
            ),
            SupportedStreamConfigRange::new(
                1,
                SampleRate(48000),
                SampleRate(48000),
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
    fn startup_errors_are_returned_and_thread_is_joined() {
        assert!(AudioStream::new("audio_test", |_| Err(Error::msg("test failure"))).is_err());
    }
}
