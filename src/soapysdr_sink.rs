//! SoapySDR sink.

use log::debug;
use std::panic::{RefUnwindSafe, UnwindSafe};

use crate::Complex;
use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::ReadStream;
use crate::{Error, Result};

fn ai_string(ai: &soapysdr::ArgInfo) -> String {
    format!(
        "key={} value={} name={:?} descr={:?} units={:?} data_type={:?} options={:?}",
        ai.key, ai.value, ai.name, ai.description, ai.units, ai.data_type, ai.options
    )
}

// Taking this callback on completion guarantees a single invocation. The trait
// bounds allow sink builders to be shared, moved, and held across catch_unwind.
type EofCallback = Box<dyn FnOnce() + Send + Sync + UnwindSafe + RefUnwindSafe>;

// SoapySDR Constants.h: SOAPY_SDR_END_BURST. The Rust wrapper exposes raw flags.
const END_BURST: i32 = 1 << 1;
const END_ABRUPT: i32 = 1 << 3;

/// TX operations used by the shared transmission loop, separable from hardware
/// so partial writes and completion events can be exercised in tests.
trait TxIo {
    /// Submit samples, marking the end of the transmission when `end` is true.
    /// Returns the accepted count; zero requests a retry without consuming input.
    fn send(&mut self, samples: &[Complex], end: bool) -> Result<usize>;
    /// Poll for confirmed burst completion. A timeout or unrelated status event
    /// returns false; unsupported reporting and transmission failures are errors.
    fn completed(&mut self) -> Result<bool>;
}

impl TxIo for soapysdr::TxStream<Complex> {
    fn send(&mut self, samples: &[Complex], end: bool) -> Result<usize> {
        match self.write(&[samples], None, end, 10_000) {
            Ok(n) => Ok(n),
            Err(e) if e.code == soapysdr::ErrorCode::Timeout => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    fn completed(&mut self) -> Result<bool> {
        let (mut channels, mut flags, mut time) = (0, 0, 0);
        match self.read_status(&mut channels, &mut flags, &mut time, 10_000) {
            Ok(_) if flags & END_ABRUPT != 0 => {
                Err(Error::msg("SoapySDR TX burst terminated prematurely"))
            }
            Ok(_) => Ok(flags & END_BURST != 0),
            Err(e) if e.code == soapysdr::ErrorCode::Timeout => Ok(false),
            Err(e) if e.code == soapysdr::ErrorCode::NotSupported => Err(Error::msg(
                "SoapySDR driver does not support TX completion reporting; cannot confirm drainage",
            )),
            Err(e) => Err(e.into()),
        }
    }
}

/// Optional EOF handling: retain a final sample, submit it with END_BURST,
/// then wait for the driver to confirm transmission before notifying the caller.
/// Dropping this state never invokes the completion callback.
struct Drain {
    callback: Option<EofCallback>,
    // Held until more input arrives or EOF proves this is the final sample.
    pending: Option<Complex>,
    // The final sample and its end marker have been accepted by the driver.
    waiting: bool,
    // Allows BlockEOF to stay false while samples or device events are pending.
    done: bool,
}

impl Drain {
    /// Complete an empty stream or a confirmed transmission. Record completion
    /// before invoking user code, and remove the callback so it cannot repeat.
    fn finish(&mut self) {
        self.done = true;
        if let Some(callback) = self.callback.take() {
            callback();
        }
    }
}

/// Shared transmission loop. Drain state enables final-sample retention and
/// completion polling; ordinary streaming submits the whole available window.
fn transmit<'a>(
    src: &'a ReadStream<Complex>,
    tx: &mut impl TxIo,
    mut drain: Option<&mut Drain>,
) -> Result<BlockRet<'a>> {
    if let Some(state) = drain.as_mut() {
        if state.done {
            return Ok(BlockRet::EOF);
        }
        if state.waiting {
            if tx.completed()? {
                state.finish();
                return Ok(BlockRet::EOF);
            }
            // Driver events are external to the graph's sample streams.
            return Ok(BlockRet::Pending);
        }
    }
    // Query before obtaining a read window, which itself retains the buffer.
    let finalize_burst = drain.is_some() && src.eof();
    let (input, _tags) = src.read_buf()?;
    if let Some(state) = drain.as_mut()
        && let Some(sample) = state.pending
    {
        if input.is_empty() && !finalize_burst {
            return Ok(BlockRet::WaitForStream(src, 1));
        }
        // A one-sample final write cannot partially accept an end marker.
        if tx.send(&[sample], finalize_burst)? == 0 {
            return Ok(BlockRet::Pending);
        }
        state.pending = None;
        state.waiting = finalize_burst;
        return Ok(BlockRet::Again);
    }
    if input.is_empty() {
        if finalize_burst {
            // Empty streams have no queued transmission to drain.
            drain.as_mut().unwrap().finish();
            return Ok(BlockRet::EOF);
        }
        return Ok(BlockRet::WaitForStream(src, 1));
    }
    if let Some(state) = drain.as_mut()
        && input.len() == 1
    {
        // Holding the sample locally allows even a capacity-one input to
        // accept more data and eventually signal EOF without deadlocking.
        state.pending = Some(input.slice()[0]);
        input.consume(1);
        return Ok(BlockRet::Again);
    }
    let available = input.len();
    // With drainage enabled, leave one sample for a possible final end marker.
    // Both modes share partial-write, timeout, and input-consumption handling.
    let count = available - usize::from(drain.is_some());
    let n = tx.send(&input.slice()[..count], false)?;
    input.consume(n);
    Ok(if n == 0 {
        BlockRet::Pending
    } else if n == available {
        BlockRet::WaitForStream(src, 1)
    } else {
        BlockRet::Again
    })
}

/// SoapySDR sink builder.
#[must_use]
pub struct SoapySdrSinkBuilder<'a> {
    dev: &'a soapysdr::Device,
    antenna: Option<String>,
    channel: usize,
    ogain: f64,
    samp_rate: f64,
    freq: f64,
    on_eof: Option<EofCallback>,
}

impl SoapySdrSinkBuilder<'_> {
    /// Set channel number.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }
    /// Set input gain.
    ///
    /// Normalized to 0.0 to 1.0.
    pub fn ogain(mut self, gain: f64) -> Result<Self> {
        if gain < 0.0 || gain > 1.0 {
            return Err(Error::msg("output gain must be in range 0.0 - 1.0"));
        }
        self.ogain = gain;
        Ok(self)
    }
    /// Set antenna.
    pub fn antenna<T: Into<String>>(mut self, a: T) -> Self {
        self.antenna = Some(a.into());
        self
    }
    /// Invoke a callback once after input EOF and confirmed TX burst completion.
    ///
    /// Enables final-sample retention and end-of-burst signalling. Driver status
    /// timeouts are retried until completion or graph cancellation. Unsupported
    /// status reporting and TX errors fail the block without invoking the hook.
    /// An empty input completes immediately. Cancellation and dropping the sink
    /// do not invoke the callback. The callback runs on the graph worker thread.
    pub fn on_eof<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + Sync + UnwindSafe + RefUnwindSafe + 'static,
    {
        self.on_eof = Some(Box::new(callback));
        self
    }

    /// Build block.
    pub fn build(self, src: ReadStream<Complex>) -> Result<SoapySdrSink> {
        let dir = soapysdr::Direction::Tx;
        debug!("SoapySDR TX driver: {}", self.dev.driver_key()?);
        debug!("SoapySDR TX hardware: {}", self.dev.hardware_key()?);
        debug!("SoapySDR TX hardware info: {}", self.dev.hardware_info()?);
        debug!(
            "SoapySDR TX frontend mapping: {}",
            self.dev.frontend_mapping(dir)?
        );
        for sensor in self.dev.list_sensors()? {
            debug!(
                "SoapySDR TX sensor {sensor}: {:?}",
                self.dev.get_sensor_info(&sensor)?
            );
            debug!(
                "SoapySDR TX sensor {sensor}: {:?}",
                self.dev.read_sensor(&sensor)?
            );
        }
        debug!(
            "SoapySDR TX clock sources: {:?}",
            self.dev.list_clock_sources()?
        );
        debug!(
            "SoapySDR TX active clock source: {:?}",
            self.dev.get_clock_source()?
        );
        let chans = self.dev.num_channels(dir)?;
        debug!("SoapySDR TX channels : {chans}");
        for channel in 0..chans {
            for sensor in self.dev.list_channel_sensors(dir, channel)? {
                match self.dev.read_channel_sensor(dir, channel, &sensor) {
                    Ok(s) => debug!("SoapySDR TX channel sensor {sensor}: {s}"),
                    Err(e) => debug!("SoapySDR TX channel sensor {sensor} error: {e}"),
                }
            }
            debug!(
                "SoapySDR TX channel {channel} antennas: {:?}",
                self.dev.antennas(dir, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} gains: {:?}",
                self.dev.list_gains(dir, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} gain range: {:?}",
                self.dev.gain_range(dir, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} frequency range: {:?}",
                self.dev.frequency_range(dir, channel)?
            );
            for ai in self.dev.stream_args_info(dir, channel)? {
                debug!("SoapySDR TX channel {channel} arg info: {}", ai_string(&ai));
            }
            debug!(
                "SoapySDR TX channel {channel} stream formats: {:?}. Native: {:?}",
                self.dev.stream_formats(dir, channel)?,
                self.dev.native_stream_format(dir, channel)?,
            );
            debug!(
                "SoapySDR TX channel {channel} info: {}",
                self.dev.channel_info(dir, channel)?
            );
        }
        let mut args = soapysdr::Args::new();
        if false {
            args.set("OFFSET", "50000");
        }
        self.dev.set_frequency(dir, self.channel, self.freq, args)?;
        self.dev
            .set_sample_rate(dir, self.channel, self.samp_rate)?;
        let gr = self.dev.gain_range(dir, self.channel)?;
        let gain = gr.minimum + self.ogain * (gr.maximum - gr.minimum);
        self.dev.set_gain(dir, self.channel, gain)?;
        if let Some(a) = self.antenna {
            self.dev.set_antenna(dir, self.channel, a)?;
        }
        let mut stream = self.dev.tx_stream(&[self.channel])?;
        stream.activate(None)?;
        Ok(SoapySdrSink {
            src,
            stream,
            drain: self.on_eof.map(|callback| Drain {
                callback: Some(callback),
                pending: None,
                waiting: false,
                done: false,
            }),
        })
    }
}

/// Transmit I/Q samples through SoapySDR.
///
/// Input stream tags are currently ignored. Timed and explicitly segmented
/// bursts are therefore not inferred from tags.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct SoapySdrSink {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    stream: soapysdr::TxStream<Complex>,
    drain: Option<Drain>,
}

impl SoapySdrSink {
    /// Create new builder.
    pub fn builder(dev: &soapysdr::Device, freq: f64, samp_rate: f64) -> SoapySdrSinkBuilder<'_> {
        SoapySdrSinkBuilder {
            dev,
            freq,
            samp_rate,
            channel: 0,
            ogain: 0.5,
            antenna: None,
            on_eof: None,
        }
    }
}

impl BlockEOF for SoapySdrSink {
    fn eof(&mut self) -> bool {
        self.drain
            .as_ref()
            .map_or_else(|| self.src.eof(), |drain| drain.done)
    }
}

impl Block for SoapySdrSink {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        transmit(&self.src, &mut self.stream, self.drain.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct FakeTx {
        samples: Vec<Complex>,
        ends: usize,
        limits: VecDeque<usize>,
        statuses: VecDeque<Result<bool>>,
    }
    impl TxIo for FakeTx {
        fn send(&mut self, samples: &[Complex], end: bool) -> Result<usize> {
            let n = self
                .limits
                .pop_front()
                .unwrap_or(samples.len())
                .min(samples.len());
            self.samples.extend_from_slice(&samples[..n]);
            if end && n != 0 {
                assert_eq!(samples.len(), 1);
                self.ends += 1;
            }
            Ok(n)
        }
        fn completed(&mut self) -> Result<bool> {
            self.statuses.pop_front().expect("unexpected status poll")
        }
    }
    fn drain() -> (Drain, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        (
            Drain {
                callback: Some(Box::new(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                })),
                pending: None,
                waiting: false,
                done: false,
            },
            calls,
        )
    }

    #[test]
    fn partial_writes_preserve_final_sample_until_confirmed_completion() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream()?;
        let samples = [Complex::new(1.0, 2.0); 5];
        let mut window = writer.write_buf()?;
        window.slice()[..5].copy_from_slice(&samples);
        window.produce(5, &[]);
        drop(writer);
        let (mut drain, calls) = drain();
        let mut tx = FakeTx {
            limits: [0, 2, 2, 0, 1].into(),
            statuses: [Ok(false), Ok(true)].into(),
            ..Default::default()
        };
        for _ in 0..20 {
            let result = transmit(&reader, &mut tx, Some(&mut drain))?;
            if matches!(result, BlockRet::EOF) {
                break;
            }
            assert_eq!(calls.load(Ordering::Relaxed), 0);
        }
        assert!(drain.done);
        assert_eq!(tx.samples, samples);
        assert_eq!(tx.ends, 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            transmit(&reader, &mut tx, Some(&mut drain))?,
            BlockRet::EOF
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn retained_sample_waits_for_more_input_or_eof() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream()?;
        let (mut drain, calls) = drain();
        let mut tx = FakeTx::default();
        let sample = Complex::new(0.5, -0.5);
        for _ in 0..2 {
            let mut window = writer.write_buf()?;
            window.slice()[0] = sample;
            window.produce(1, &[]);
            for _ in 0..2 {
                transmit(&reader, &mut tx, Some(&mut drain))?;
            }
            assert!(matches!(
                transmit(&reader, &mut tx, Some(&mut drain))?,
                BlockRet::WaitForStream(_, 1)
            ));
            assert_eq!(tx.ends, 0);
        }
        drop(writer);
        tx.statuses.push_back(Ok(true));
        transmit(&reader, &mut tx, Some(&mut drain))?;
        assert!(matches!(
            transmit(&reader, &mut tx, Some(&mut drain))?,
            BlockRet::EOF
        ));
        assert_eq!(tx.samples, [sample, sample]);
        assert_eq!(tx.ends, 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn completion_errors_and_cancellation_do_not_call_hook() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream::<Complex>()?;
        drop(writer);
        let (mut drain, calls) = drain();
        drain.waiting = true;
        let mut tx = FakeTx {
            statuses: [Err(Error::msg("unsupported TX completion"))].into(),
            ..Default::default()
        };
        assert!(transmit(&reader, &mut tx, Some(&mut drain)).is_err());
        assert!(!drain.done);
        drop(drain);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[test]
    fn streaming_without_hook_uses_full_window_and_retries_partial_writes() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream()?;
        let samples = [Complex::new(1.0, -1.0); 3];
        let mut window = writer.write_buf()?;
        window.slice()[..3].copy_from_slice(&samples);
        window.produce(3, &[]);
        let mut tx = FakeTx {
            limits: [0, 1, 2].into(),
            ..Default::default()
        };
        assert!(matches!(
            transmit(&reader, &mut tx, None)?,
            BlockRet::Pending
        ));
        assert!(matches!(transmit(&reader, &mut tx, None)?, BlockRet::Again));
        assert!(matches!(
            transmit(&reader, &mut tx, None)?,
            BlockRet::WaitForStream(_, 1)
        ));
        assert_eq!(tx.samples, samples);
        assert_eq!(tx.ends, 0);
        drop(writer);
        assert!(matches!(
            transmit(&reader, &mut tx, None)?,
            BlockRet::WaitForStream(_, 1)
        ));
        Ok(())
    }

    #[test]
    fn empty_stream_completes_without_device_io() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream::<Complex>()?;
        drop(writer);
        let (mut drain, calls) = drain();
        let mut tx = FakeTx::default();
        assert!(matches!(
            transmit(&reader, &mut tx, Some(&mut drain))?,
            BlockRet::EOF
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(tx.ends, 0);
        Ok(())
    }
}
