//! Capture audio from an input device as mono floating-point samples.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::audio::{AudioStream, device_config};
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream, new_stream};
use crate::{Error, Float, Result};

struct Capture {
    receiver: Receiver<Float>,
    overrun: Arc<AtomicBool>,
    last_overrun_warning: Option<Instant>,
}

impl Capture {
    fn work<'a>(&mut self, dst: &'a WriteStream<Float>) -> Result<BlockRet<'a>> {
        // Report loss from the worker, never from the audio callback. Clear
        // the flag so later callbacks can recover without stopping the graph.
        if self.overrun.swap(false, Ordering::Relaxed)
            && self
                .last_overrun_warning
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(5))
        {
            log::warn!("Audio input buffer overrun; dropped capture samples");
            self.last_overrun_warning = Some(Instant::now());
        }
        let mut output = dst.write_buf()?;
        if output.is_empty() {
            return Ok(BlockRet::WaitForStream(dst, 1));
        }
        let mut n = 0;
        let mut closed = false;
        for sample in output.slice() {
            match self.receiver.try_recv() {
                Ok(value) => {
                    *sample = value;
                    n += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        output.produce(n, &[]);
        Ok(if n != 0 {
            BlockRet::Again
        } else if closed {
            BlockRet::EOF
        } else {
            BlockRet::Pending
        })
    }
}

fn capture<T>(data: &[T], channels: usize, sender: &SyncSender<Float>, overrun: &AtomicBool)
where
    T: Sample + Copy,
    Float: FromSample<T>,
{
    for frame in data.chunks_exact(channels) {
        // Average channels so mono capture also works with stereo cables.
        let value = frame.iter().map(|&v| Float::from_sample(v)).sum::<Float>() / channels as Float;
        match sender.try_send(value) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                overrun.store(true, Ordering::Relaxed);
                return;
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

fn start<T>(
    device: cpal::Device,
    config: cpal::StreamConfig,
    sender: SyncSender<Float>,
    overrun: Arc<AtomicBool>,
    errors: std::sync::mpsc::Sender<cpal::Error>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Copy,
    Float: FromSample<T>,
{
    let channels = config.channels as usize;
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| capture(data, channels, &sender, &overrun),
        move |e| {
            let _ = errors.send(e);
        },
        None,
    )?)
}

/// Builder for audio capture.
#[derive(Default)]
pub struct AudioSourceBuilder {
    device: Option<String>,
}

impl AudioSourceBuilder {
    /// Create a builder using the default input device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Select the first input device whose name contains this string.
    #[must_use]
    pub fn device(mut self, name: impl Into<String>) -> Self {
        self.device = Some(name.into());
        self
    }

    /// Open capture at the requested sample rate, in Hz.
    ///
    /// Returns errors for invalid rates, missing devices, unsupported formats
    /// or rates, stream allocation, and device startup failures.
    pub fn build(self, sample_rate: u64) -> Result<(AudioSource, ReadStream<Float>)> {
        let (device, config) = device_config(sample_rate, self.device.as_deref(), true)?;
        let (dst, reader) = new_stream()?;
        let (sender, receiver) = sync_channel((sample_rate as usize / 5).max(1));
        let overrun = Arc::new(AtomicBool::new(false));
        let capture = Capture {
            receiver,
            overrun: overrun.clone(),
            last_overrun_warning: None,
        };
        let audio = AudioStream::new("audio_source_stream", move |errors| {
            match config.sample_format() {
                SampleFormat::F32 => start::<f32>(device, config.into(), sender, overrun, errors),
                SampleFormat::I16 => start::<i16>(device, config.into(), sender, overrun, errors),
                SampleFormat::U16 => start::<u16>(device, config.into(), sender, overrun, errors),
                _ => Err(Error::msg("unsupported audio input sample format")),
            }
        })?;
        dst.initialize_sample_rate(Some(sample_rate as f64));
        Ok((
            AudioSource {
                dst,
                capture,
                audio,
            }
            .with_output_metadata(),
            reader,
        ))
    }
}

/// Audio capture, producing normalized mono `Float` samples without tags.
///
/// Multichannel devices are averaged to mono. Supports f32, i16, and u16
/// device formats. Capture uses a bounded 200 ms queue. On overrun, excess
/// samples are dropped and capture continues; the worker warns at most once
/// every five seconds. The callback never waits for space. Device errors are
/// reported by `work()`.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct AudioSource {
    #[rustradio(out)]
    dst: WriteStream<Float>,
    capture: Capture,
    audio: AudioStream,
}

impl AudioSource {
    /// Capture from the default input device at `sample_rate` Hz.
    pub fn new(sample_rate: u64) -> Result<(Self, ReadStream<Float>)> {
        Self::builder().build(sample_rate)
    }

    /// Select a capture device before opening it.
    #[must_use]
    pub fn builder() -> AudioSourceBuilder {
        AudioSourceBuilder::default()
    }
}

impl Block for AudioSource {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        self.audio.play()?;
        self.audio.check_error()?;
        self.capture.work(&self.dst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stereo_and_drains_before_eof() -> Result<()> {
        let (sender, receiver) = sync_channel(4);
        let mut capture_state = Capture {
            receiver,
            overrun: Arc::new(AtomicBool::new(false)),
            last_overrun_warning: None,
        };
        let (dst, reader) = new_stream()?;
        assert!(matches!(capture_state.work(&dst)?, BlockRet::Pending));
        capture(
            &[i16::MIN, i16::MIN, 0, 0, i16::MAX, i16::MIN],
            2,
            &sender,
            &capture_state.overrun,
        );
        drop(sender);
        assert!(matches!(capture_state.work(&dst)?, BlockRet::Again));
        assert!(matches!(capture_state.work(&dst)?, BlockRet::EOF));
        let (buf, tags) = reader.read_buf()?;
        assert!(tags.is_empty());
        assert_eq!(buf.len(), 3);
        assert_eq!(buf[0], -1.0);
        assert_eq!(buf[1], 0.0);
        assert!(buf[2].abs() < 0.0001);
        Ok(())
    }

    #[test]
    fn recovers_from_overrun_and_preserves_queued_samples_under_backpressure() -> Result<()> {
        let (sender, receiver) = sync_channel(1);
        let mut state = Capture {
            receiver,
            overrun: Arc::new(AtomicBool::new(false)),
            last_overrun_warning: None,
        };
        let (dst, reader) = new_stream()?;
        let mut output = dst.write_buf()?;
        let n = output.len();
        output.slice().fill(0.0);
        output.produce(n, &[]);
        capture(&[0.5f32], 1, &sender, &state.overrun);
        assert!(matches!(state.work(&dst)?, BlockRet::WaitForStream(_, 1)));
        reader.read_buf()?.0.consume(n);
        assert!(matches!(state.work(&dst)?, BlockRet::Again));
        assert_eq!(reader.read_buf()?.0.slice(), &[0.5]);
        capture(&[0.1f32, 0.2], 1, &sender, &state.overrun);
        assert!(matches!(state.work(&dst)?, BlockRet::Again));
        assert!(!state.overrun.load(Ordering::Relaxed));
        let warned = state.last_overrun_warning;
        assert!(warned.is_some());
        let (buf, _) = reader.read_buf()?;
        assert_eq!(buf.slice(), &[0.5, 0.1]);
        buf.consume(2);
        // A later callback is accepted, and repeated overruns are throttled.
        capture(&[0.3f32, 0.4], 1, &sender, &state.overrun);
        assert!(matches!(state.work(&dst)?, BlockRet::Again));
        assert_eq!(reader.read_buf()?.0.slice(), &[0.3]);
        assert_eq!(state.last_overrun_warning, warned);
        Ok(())
    }
}
