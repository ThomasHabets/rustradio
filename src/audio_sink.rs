//! Play mono floating-point samples through an audio output device.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use cpal::traits::DeviceTrait;
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::audio::{AudioStream, device_config};
use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::ReadStream;
use crate::{Error, Float, Result};

struct Playback {
    sender: SyncSender<Float>,
    queued: Arc<AtomicUsize>,
}

impl Playback {
    fn work<'a>(&self, src: &'a ReadStream<Float>) -> Result<BlockRet<'a>> {
        let (input, _tags) = src.read_buf()?;
        let mut n = 0;
        for &sample in input.iter() {
            // Reserve before sending: the callback can receive immediately.
            self.queued.fetch_add(1, Ordering::Relaxed);
            match self.sender.try_send(sample) {
                Ok(()) => n += 1,
                Err(TrySendError::Full(_)) => {
                    self.queued.fetch_sub(1, Ordering::Relaxed);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.queued.fetch_sub(1, Ordering::Relaxed);
                    input.consume(n);
                    return Err(Error::msg("audio output disconnected"));
                }
            }
        }
        let remaining = input.len() - n;
        input.consume(n);
        Ok(if n != 0 {
            BlockRet::Again
        } else if remaining != 0 || self.queued.load(Ordering::Relaxed) != 0 {
            BlockRet::Pending
        } else {
            BlockRet::WaitForStream(src, 1)
        })
    }
}

fn play<T>(data: &mut [T], channels: usize, receiver: &Receiver<Float>, queued: &AtomicUsize)
where
    T: Sample + FromSample<Float>,
{
    for frame in data.chunks_mut(channels) {
        let value = match receiver.try_recv() {
            Ok(value) => {
                queued.fetch_sub(1, Ordering::Relaxed);
                value
            }
            Err(_) => 0.0,
        };
        frame.fill(T::from_sample(value));
    }
}

fn start<T>(
    device: cpal::Device,
    config: cpal::StreamConfig,
    receiver: Receiver<Float>,
    queued: Arc<AtomicUsize>,
    errors: std::sync::mpsc::Sender<cpal::StreamError>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<Float>,
{
    let channels = config.channels as usize;
    Ok(device.build_output_stream(
        &config,
        move |data: &mut [T], _| play(data, channels, &receiver, &queued),
        move |e| {
            let _ = errors.send(e);
        },
        None,
    )?)
}

/// Audio sink builder.
#[derive(Default)]
pub struct AudioSinkBuilder {
    dev: Option<String>,
}

impl AudioSinkBuilder {
    /// Create a builder using the default output device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Select the first output device whose name contains this string.
    #[must_use]
    pub fn device(mut self, name: impl Into<String>) -> Self {
        self.dev = Some(name.into());
        self
    }

    /// Open playback at the requested sample rate, in Hz.
    ///
    /// Returns errors for invalid rates, missing devices, unsupported formats
    /// or rates, and device startup failures.
    pub fn build(self, prev: ReadStream<Float>, sample_rate: u64) -> Result<AudioSink> {
        AudioSink::new_opts(prev, sample_rate, self.dev.as_deref())
    }
}

/// Audio playback of mono samples in the range -1.0 to +1.0.
///
/// Mono is copied to every device channel. Supports f32, i16, and u16 device
/// formats. The callback outputs silence when no samples are queued, and never
/// waits for input. Tags are discarded. Playback uses a bounded 200 ms queue;
/// queued samples drain before input EOF completes the block. Device errors
/// are reported by `work()`.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct AudioSink {
    #[rustradio(in)]
    src: ReadStream<Float>,
    playback: Playback,
    audio: AudioStream,
}

impl AudioSink {
    /// Play mono audio through the default output device at `sample_rate` Hz.
    pub fn new(src: ReadStream<Float>, sample_rate: u64) -> Result<Self> {
        Self::new_opts(src, sample_rate, None)
    }

    /// Select an output device before opening it.
    #[must_use]
    pub fn builder() -> AudioSinkBuilder {
        AudioSinkBuilder::default()
    }

    fn new_opts(src: ReadStream<Float>, sample_rate: u64, dev: Option<&str>) -> Result<Self> {
        let (device, config) = device_config(sample_rate, dev, false)?;
        let (sender, receiver) = sync_channel((sample_rate as usize / 5).max(1));
        let queued = Arc::new(AtomicUsize::new(0));
        let playback = Playback {
            sender,
            queued: queued.clone(),
        };
        let mut audio = AudioStream::new("audio_sink_stream", move |errors| {
            match config.sample_format() {
                SampleFormat::F32 => start::<f32>(device, config.into(), receiver, queued, errors),
                SampleFormat::I16 => start::<i16>(device, config.into(), receiver, queued, errors),
                SampleFormat::U16 => start::<u16>(device, config.into(), receiver, queued, errors),
                _ => Err(Error::msg("unsupported audio output sample format")),
            }
        })?;
        audio.play()?;
        Ok(Self {
            src,
            playback,
            audio,
        })
    }
}

impl BlockEOF for AudioSink {
    fn eof(&mut self) -> bool {
        self.src.eof() && self.playback.queued.load(Ordering::Relaxed) == 0
    }
}

impl Block for AudioSink {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        self.audio.check_error()?;
        self.playback.work(&self.src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_stream;

    #[test]
    fn is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AudioSink>();
    }

    #[test]
    fn silence_and_stereo_playback_preserve_samples_under_backpressure() -> Result<()> {
        let (sender, receiver) = sync_channel(2);
        let state = Playback {
            sender,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        let (writer, reader) = new_stream()?;
        let mut output = [1.0f32; 6];
        play(&mut output, 2, &receiver, &state.queued);
        assert_eq!(output, [0.0; 6]);
        let mut window = writer.write_buf()?;
        window.slice()[..3].copy_from_slice(&[0.25, -0.5, 0.75]);
        window.produce(3, &[]);
        drop(writer);
        assert!(matches!(state.work(&reader)?, BlockRet::Again));
        assert_eq!(state.queued.load(Ordering::Relaxed), 2);
        assert!(matches!(state.work(&reader)?, BlockRet::Pending));
        assert_eq!(reader.read_buf()?.0.slice(), &[0.75]);
        play(&mut output, 2, &receiver, &state.queued);
        assert_eq!(output, [0.25, 0.25, -0.5, -0.5, 0.0, 0.0]);
        assert!(matches!(state.work(&reader)?, BlockRet::Again));
        assert!(matches!(state.work(&reader)?, BlockRet::Pending));
        play(&mut output, 2, &receiver, &state.queued);
        assert_eq!(output, [0.75, 0.75, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(state.queued.load(Ordering::Relaxed), 0);
        assert!(matches!(
            state.work(&reader)?,
            BlockRet::WaitForStream(_, 1)
        ));
        Ok(())
    }

    #[test]
    fn unsigned_silence_uses_equilibrium_and_disconnect_is_reported() -> Result<()> {
        let (sender, receiver) = sync_channel(1);
        let state = Playback {
            sender,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        let mut data = [0u16; 2];
        play(&mut data, 1, &receiver, &state.queued);
        assert_eq!(data, [32768; 2]);
        drop(receiver);
        let (writer, reader) = new_stream()?;
        let mut window = writer.write_buf()?;
        window.slice()[0] = 0.1;
        window.produce(1, &[]);
        assert!(state.work(&reader).is_err());
        assert_eq!(state.queued.load(Ordering::Relaxed), 0);
        Ok(())
    }
}
