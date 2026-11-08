//! Play mono floating-point samples through an audio output device.
use std::panic::{RefUnwindSafe, UnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::audio::{AudioStream, device_config};
use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::ReadStream;
use crate::{Error, Float, Result};

// Shared by the graph worker and the device callback. An empty channel only
// means samples have reached the backend; they may still be waiting to play.
struct PlaybackStatus {
    // Includes samples reserved by the worker and samples being copied by the
    // current callback. The callback retires them after publishing its deadline.
    queued: AtomicUsize,
    // Common monotonic clock for the callback's deadline and the worker's poll.
    origin: Instant,
    // Nanoseconds since origin, at the estimated end of the last graph sample.
    until: AtomicU64,
}

impl Default for PlaybackStatus {
    fn default() -> Self {
        Self {
            queued: AtomicUsize::new(0),
            origin: Instant::now(),
            until: AtomicU64::new(0),
        }
    }
}

impl PlaybackStatus {
    fn idle(&self) -> bool {
        // The Acquire load pairs with the callback's queue decrement: seeing
        // that decrement also makes its preceding deadline update visible.
        self.queued.load(Ordering::Acquire) == 0
            && self.origin.elapsed().as_nanos() >= self.until.load(Ordering::Acquire) as u128
    }
}

// The bounds allow builders to be sent, shared, and held across catch_unwind.
type TransmitCallback =
    Box<dyn FnMut(bool) -> Result<()> + Send + Sync + UnwindSafe + RefUnwindSafe>;
type EofCallback = Box<dyn FnOnce() + Send + Sync + UnwindSafe + RefUnwindSafe>;

// Only the graph worker invokes this callback during normal processing. Control
// may do blocking I/O or wait for radio setup without holding up the audio thread.
struct TransmitControl {
    callback: TransmitCallback,
    active: bool,
}

impl TransmitControl {
    fn start(&mut self) -> Result<()> {
        if !self.active {
            // A failed acknowledgement can still mean the transmitter keyed.
            self.active = true;
            (self.callback)(true)?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if self.active {
            (self.callback)(false)?;
            // Keep active on failure so Drop can make another release attempt.
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for TransmitControl {
    fn drop(&mut self) {
        // Covers cancellation and processing errors as well as normal EOF.
        // Drop cannot propagate an error, so release failures are logged.
        if let Err(e) = self.stop() {
            log::error!("Failed to release audio transmit control: {e}");
        }
    }
}

struct Playback {
    sender: SyncSender<Float>,
    status: Arc<PlaybackStatus>,
}

impl Playback {
    fn eof(
        &self,
        src: &ReadStream<Float>,
        control: Option<&TransmitControl>,
        on_eof: &mut Option<EofCallback>,
    ) -> bool {
        // Completion includes device playback and key-off, not just input EOF.
        let eof = src.eof() && self.status.idle() && control.is_none_or(|c| !c.active);
        if eof && let Some(callback) = on_eof.take() {
            // EOF may be queried repeatedly; taking the callback makes it once-only.
            callback();
        }
        eof
    }

    fn work<'a>(
        &self,
        src: &'a ReadStream<Float>,
        mut control: Option<&mut TransmitControl>,
    ) -> Result<BlockRet<'a>> {
        let (input, _tags) = src.read_buf()?;
        if input.is_empty() {
            return if self.status.idle() {
                if let Some(control) = control {
                    control.stop()?;
                }
                Ok(BlockRet::WaitForStream(src, 1))
            } else {
                // Audio consumption and the passage of the playback deadline
                // happen outside graph streams, so poll rather than wait on src.
                Ok(BlockRet::Pending)
            };
        }
        if let Some(control) = control.as_mut() {
            // Finish key-on and any configured lead delay before making even
            // the first sample available to the device callback.
            control.start()?;
        }
        let mut n = 0;
        for &sample in input.iter() {
            // Reserve before sending: the callback can receive immediately.
            self.status.queued.fetch_add(1, Ordering::Relaxed);
            match self.sender.try_send(sample) {
                Ok(()) => n += 1,
                Err(TrySendError::Full(_)) => {
                    self.status.queued.fetch_sub(1, Ordering::Relaxed);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.status.queued.fetch_sub(1, Ordering::Relaxed);
                    input.consume(n);
                    return Err(Error::msg("audio output disconnected"));
                }
            }
        }
        input.consume(n);
        // A full audio channel is relieved by the device callback, not by
        // activity on src. Consume only the samples successfully enqueued.
        Ok(if n != 0 {
            BlockRet::Again
        } else {
            BlockRet::Pending
        })
    }
}

// Runs on the audio thread: fill every device frame without waiting for the
// graph, and report when the final queued sample should finish playing.
fn play<T>(
    data: &mut [T],
    channels: usize,
    receiver: &Receiver<Float>,
    status: &PlaybackStatus,
    rate: u32,
    latency: Duration,
) where
    T: Sample + FromSample<Float>,
{
    let now = status.origin.elapsed();
    let mut consumed = 0;
    let mut last_frame = 0;
    for (pos, frame) in data.chunks_mut(channels).enumerate() {
        let value = match receiver.try_recv() {
            Ok(value) => {
                consumed += 1;
                // Count mono frames, independent of the device channel count.
                // Silence inserted after the final sample does not extend PTT.
                last_frame = pos + 1;
                value
            }
            Err(_) => 0.0,
        };
        frame.fill(T::from_sample(value));
    }
    if consumed != 0 {
        let until = now + latency + Duration::from_secs_f64(last_frame as f64 / rate as f64);
        status.until.store(
            u64::try_from(until.as_nanos()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
        // Publish the deadline before allowing the worker to see an empty queue.
        // Otherwise it could release PTT while these samples remain in the
        // device buffer. One decrement accounts for this callback's whole batch.
        status.queued.fetch_sub(consumed, Ordering::Release);
    }
}

fn start<T>(
    device: cpal::Device,
    config: cpal::StreamConfig,
    receiver: Receiver<Float>,
    status: Arc<PlaybackStatus>,
    errors: std::sync::mpsc::Sender<cpal::StreamError>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<Float>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;
    Ok(device.build_output_stream(
        &config,
        move |data: &mut [T], info| {
            let timestamp = info.timestamp();
            // CPAL estimates when the first frame will reach the device. Use
            // the difference between CPAL timestamps as a duration; their clock
            // cannot be directly compared with std::time::Instant.
            let latency = timestamp
                .playback
                .duration_since(&timestamp.callback)
                .unwrap_or_default();
            play(data, channels, &receiver, &status, rate, latency);
        },
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
    transmit: Option<TransmitControl>,
    on_eof: Option<EofCallback>,
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

    /// Set a transmit control callback, called on the graph worker thread.
    ///
    /// `true` is called before queuing a burst's first sample, and may wait for
    /// transmitter setup. `false` is called when input and playback queues are
    /// empty and the backend's estimated playback end has passed. Consecutive
    /// queued packets share one transmission. Callback errors stop the block;
    /// dropping an active control also attempts `false` and logs any failure.
    /// The callback is never called from the audio thread. Additional external
    /// buffering can be accommodated by waiting inside the `false` callback.
    #[must_use]
    pub fn transmit_control<F>(mut self, callback: F) -> Self
    where
        F: FnMut(bool) -> Result<()> + Send + Sync + UnwindSafe + RefUnwindSafe + 'static,
    {
        self.transmit = Some(TransmitControl {
            callback: Box::new(callback),
            active: false,
        });
        self
    }

    /// Run a callback once when EOF is detected after input, device playback,
    /// and transmit control have all finished.
    ///
    /// Runs on the thread detecting EOF, never on the audio callback thread.
    /// Cancellation, errors, and dropping the sink do not invoke this callback.
    #[must_use]
    pub fn on_eof<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + Sync + UnwindSafe + RefUnwindSafe + 'static,
    {
        self.on_eof = Some(Box::new(callback));
        self
    }

    /// Open playback at the requested sample rate, in Hz.
    ///
    /// Returns errors for invalid rates, missing devices, unsupported formats
    /// or rates, and device startup failures.
    pub fn build(self, prev: ReadStream<Float>, sample_rate: u64) -> Result<AudioSink> {
        AudioSink::new_opts(
            prev,
            sample_rate,
            self.dev.as_deref(),
            self.transmit,
            self.on_eof,
        )
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
    // Fields drop in declaration order: stop the audio stream before attempting
    // to release an active transmitter during cancellation or error cleanup.
    transmit: Option<TransmitControl>,
    on_eof: Option<EofCallback>,
}

impl AudioSink {
    /// Play mono audio through the default output device at `sample_rate` Hz.
    pub fn new(src: ReadStream<Float>, sample_rate: u64) -> Result<Self> {
        Self::new_opts(src, sample_rate, None, None, None)
    }

    /// Select an output device before opening it.
    #[must_use]
    pub fn builder() -> AudioSinkBuilder {
        AudioSinkBuilder::default()
    }

    fn new_opts(
        src: ReadStream<Float>,
        sample_rate: u64,
        dev: Option<&str>,
        transmit: Option<TransmitControl>,
        on_eof: Option<EofCallback>,
    ) -> Result<Self> {
        let (device, config) = device_config(sample_rate, dev, false)?;
        let (sender, receiver) = sync_channel((sample_rate as usize / 5).max(1));
        let status = Arc::new(PlaybackStatus::default());
        let playback = Playback {
            sender,
            status: status.clone(),
        };
        let mut audio = AudioStream::new("audio_sink_stream", move |errors| {
            match config.sample_format() {
                SampleFormat::F32 => start::<f32>(device, config.into(), receiver, status, errors),
                SampleFormat::I16 => start::<i16>(device, config.into(), receiver, status, errors),
                SampleFormat::U16 => start::<u16>(device, config.into(), receiver, status, errors),
                _ => Err(Error::msg("unsupported audio output sample format")),
            }
        })?;
        audio.play()?;
        Ok(Self {
            src,
            playback,
            audio,
            transmit,
            on_eof,
        })
    }
}

impl BlockEOF for AudioSink {
    fn eof(&mut self) -> bool {
        // The worker still needs to run after input EOF to drain playback and
        // invoke key-off. Neither handing samples to CPAL nor input EOF suffices.
        self.playback
            .eof(&self.src, self.transmit.as_ref(), &mut self.on_eof)
    }
}

impl Block for AudioSink {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        self.audio.check_error()?;
        self.playback.work(&self.src, self.transmit.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_stream;

    #[test]
    fn is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        fn assert_unwind_safe<T: UnwindSafe + RefUnwindSafe>() {}
        assert_send_sync::<AudioSink>();
        assert_send_sync::<AudioSinkBuilder>();
        assert_unwind_safe::<AudioSinkBuilder>();
    }

    #[test]
    fn transmit_control_covers_queued_packets_and_device_latency() -> Result<()> {
        use std::sync::Mutex;

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        let worker = std::thread::current().id();
        let mut control = TransmitControl {
            callback: Box::new(move |on| {
                assert_eq!(std::thread::current().id(), worker);
                observed.lock().unwrap().push(on);
                Ok(())
            }),
            active: false,
        };
        let (sender, receiver) = sync_channel(4);
        let playback = Playback {
            sender,
            status: Arc::new(PlaybackStatus::default()),
        };
        let (writer, reader) = new_stream()?;
        let completions = Arc::new(AtomicUsize::new(0));
        let completed = completions.clone();
        let completion_events = events.clone();
        let mut on_eof: Option<EofCallback> = Some(Box::new(move || {
            assert_eq!(*completion_events.lock().unwrap(), [true, false]);
            completed.fetch_add(1, Ordering::Relaxed);
        }));
        assert!(!playback.eof(&reader, Some(&control), &mut on_eof));
        assert!(matches!(
            playback.work(&reader, Some(&mut control))?,
            BlockRet::WaitForStream(_, 1)
        ));
        assert!(events.lock().unwrap().is_empty());
        for value in [0.25, -0.5] {
            let mut window = writer.write_buf()?;
            window.slice()[0] = value;
            window.produce(1, &[]);
            assert!(matches!(
                playback.work(&reader, Some(&mut control))?,
                BlockRet::Again
            ));
        }
        drop(writer);
        assert_eq!(*events.lock().unwrap(), [true]);
        assert!(!playback.eof(&reader, Some(&control), &mut on_eof));
        let mut output = [1.0f32; 4];
        play(
            &mut output,
            1,
            &receiver,
            &playback.status,
            48000,
            Duration::from_secs(60),
        );
        assert_eq!(output, [0.25, -0.5, 0.0, 0.0]);
        assert_eq!(playback.status.queued.load(Ordering::Acquire), 0);
        assert!(matches!(
            playback.work(&reader, Some(&mut control))?,
            BlockRet::Pending
        ));
        assert_eq!(*events.lock().unwrap(), [true]);
        assert!(!playback.eof(&reader, Some(&control), &mut on_eof));
        // Simulate passing the backend's playback deadline without sleeping.
        playback.status.until.store(0, Ordering::Release);
        assert!(!playback.eof(&reader, Some(&control), &mut on_eof));
        assert!(matches!(
            playback.work(&reader, Some(&mut control))?,
            BlockRet::WaitForStream(_, 1)
        ));
        assert_eq!(*events.lock().unwrap(), [true, false]);
        assert!(!control.active);
        assert!(playback.eof(&reader, Some(&control), &mut on_eof));
        assert!(playback.eof(&reader, Some(&control), &mut on_eof));
        assert_eq!(completions.load(Ordering::Relaxed), 1);
        drop(control);
        assert_eq!(*events.lock().unwrap(), [true, false]);
        Ok(())
    }

    #[test]
    fn empty_input_eof_notifies_once() -> Result<()> {
        let (sender, _receiver) = sync_channel(1);
        let playback = Playback {
            sender,
            status: Arc::new(PlaybackStatus::default()),
        };
        let (writer, reader) = new_stream()?;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let mut on_eof: Option<EofCallback> = Some(Box::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        assert!(!playback.eof(&reader, None, &mut on_eof));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        drop(writer);
        assert!(playback.eof(&reader, None, &mut on_eof));
        assert!(playback.eof(&reader, None, &mut on_eof));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn failed_key_on_preserves_input_and_drop_attempts_key_off() -> Result<()> {
        use std::sync::Mutex;

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        let mut control = TransmitControl {
            callback: Box::new(move |on| {
                observed.lock().unwrap().push(on);
                if on {
                    Err(Error::msg("lost acknowledgement"))
                } else {
                    Ok(())
                }
            }),
            active: false,
        };
        let (sender, _receiver) = sync_channel(4);
        let playback = Playback {
            sender,
            status: Arc::new(PlaybackStatus::default()),
        };
        let (writer, reader) = new_stream()?;
        let mut window = writer.write_buf()?;
        window.slice()[0] = 0.5;
        window.produce(1, &[]);
        assert!(playback.work(&reader, Some(&mut control)).is_err());
        assert_eq!(reader.read_buf()?.0.slice(), &[0.5]);
        assert_eq!(playback.status.queued.load(Ordering::Acquire), 0);
        drop(control);
        assert_eq!(*events.lock().unwrap(), [true, false]);
        Ok(())
    }

    #[test]
    fn silence_and_stereo_playback_preserve_samples_under_backpressure() -> Result<()> {
        let (sender, receiver) = sync_channel(2);
        let state = Playback {
            sender,
            status: Arc::new(PlaybackStatus::default()),
        };
        let (writer, reader) = new_stream()?;
        let mut output = [1.0f32; 6];
        play(
            &mut output,
            2,
            &receiver,
            &state.status,
            48000,
            Duration::ZERO,
        );
        assert_eq!(output, [0.0; 6]);
        let mut window = writer.write_buf()?;
        window.slice()[..3].copy_from_slice(&[0.25, -0.5, 0.75]);
        window.produce(3, &[]);
        drop(writer);
        assert!(matches!(state.work(&reader, None)?, BlockRet::Again));
        assert_eq!(state.status.queued.load(Ordering::Relaxed), 2);
        assert!(matches!(state.work(&reader, None)?, BlockRet::Pending));
        assert_eq!(reader.read_buf()?.0.slice(), &[0.75]);
        play(
            &mut output,
            2,
            &receiver,
            &state.status,
            48000,
            Duration::ZERO,
        );
        assert_eq!(output, [0.25, 0.25, -0.5, -0.5, 0.0, 0.0]);
        assert!(matches!(state.work(&reader, None)?, BlockRet::Again));
        assert!(matches!(state.work(&reader, None)?, BlockRet::Pending));
        play(
            &mut output,
            2,
            &receiver,
            &state.status,
            48000,
            Duration::ZERO,
        );
        assert_eq!(output, [0.75, 0.75, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(state.status.queued.load(Ordering::Relaxed), 0);
        state.status.until.store(0, Ordering::Release);
        assert!(matches!(
            state.work(&reader, None)?,
            BlockRet::WaitForStream(_, 1)
        ));
        Ok(())
    }

    #[test]
    fn unsigned_silence_uses_equilibrium_and_disconnect_is_reported() -> Result<()> {
        let (sender, receiver) = sync_channel(1);
        let state = Playback {
            sender,
            status: Arc::new(PlaybackStatus::default()),
        };
        let mut data = [0u16; 2];
        play(
            &mut data,
            1,
            &receiver,
            &state.status,
            48000,
            Duration::ZERO,
        );
        assert_eq!(data, [32768; 2]);
        drop(receiver);
        let (writer, reader) = new_stream()?;
        let mut window = writer.write_buf()?;
        window.slice()[0] = 0.1;
        window.produce(1, &[]);
        assert!(state.work(&reader, None).is_err());
        assert_eq!(state.status.queued.load(Ordering::Relaxed), 0);
        Ok(())
    }
}
