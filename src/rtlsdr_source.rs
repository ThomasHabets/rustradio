/*! RTL SDR source.

RTL-SDRs are the most common type of SDR hardware. They're cheap, and
good for up to about 2.8Msps (2.8Mhz slice of spectrum) from about
24MHz to 1.75Ghz.

They can't transmit, but are good for most beginner receiver use
cases.

The best places to get RTL SDRs are probably:
* <https://www.rtl-sdr.com>
* <https://www.nooelec.com/store/>
*/
use std::sync::mpsc;
use std::sync::mpsc::{SendError, TryRecvError};
use std::thread;

use log::{debug, warn};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Error, Result};

const CHUNK_SIZE: usize = 8192;
const MAX_CHUNKS_IN_FLIGHT: usize = 1000;

#[derive(Debug, Clone)]
pub struct RtlSdrControl {
    tx: mpsc::Sender<RtlSdrCommand>,
}

#[derive(Debug)]
enum RtlSdrCommand {
    CenterFreqHz(u32),
    TunerGain(i32),
    SampleRate(u32),
}

impl RtlSdrControl {
    /// Retune the RTL-SDR center frequency (in Hz) without rebuilding the graph.
    pub fn set_center_freq_hz(&self, hz: u32) -> Result<()> {
        self.tx.send(RtlSdrCommand::CenterFreqHz(hz))?;
        Ok(())
    }

    /// Set tuner gain (in dB) without rebuilding the graph.
    pub fn set_gain_db(&self, gain_db: i32) -> Result<()> {
        self.tx.send(RtlSdrCommand::TunerGain(gain_db))?;
        Ok(())
    }

    /// Set sample rate (in samples per second) without rebuilding the graph.
    pub fn set_sample_rate(&self, samp_rate: u32) -> Result<()> {
        self.tx.send(RtlSdrCommand::SampleRate(samp_rate))?;
        Ok(())
    }
}

impl From<rtlsdr::RTLSDRError> for Error {
    fn from(e: rtlsdr::RTLSDRError) -> Self {
        // For some reason RTLSDRError doesn't implement Error.
        Error::device(Error::msg(format!("{e}")), "rtlsdr")
    }
}

impl<T: Send + Sync + 'static> From<SendError<T>> for Error {
    fn from(e: SendError<T>) -> Self {
        // Macro above doesn't deal with generics.
        Error::device(e, "RTL-SDR")
    }
}

/// RTL SDR Source block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct RtlSdrSource {
    rx: mpsc::Receiver<Vec<u8>>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    buf: Vec<u8>,
    control_tx: RtlSdrControl,
}

impl RtlSdrSource {
    /// Create new RtlSdrSource block.
    ///
    /// * `freq`: Center frequency, in Hz.
    /// * `samp_rate`: samples per second. Equivalently, the bandwidth.
    /// * `igain`: Input gain. 20 is a good number to start with.
    ///
    /// If given frequency of 100Mhz, and sample rate of 1Msps, the
    /// received spectrum is 99.5Mhz to 100.5Mhz.
    pub fn new(freq: u64, samp_rate: u32, igain: i32) -> Result<(Self, ReadStream<u8>)> {
        let index = 0;
        let found = rtlsdr::get_device_count();
        if index >= found {
            return Err(Error::msg(format!(
                "RTL SDR index {index} doesn't exist, found {found}"
            )));
        }

        let (tx, rx) = mpsc::sync_channel(MAX_CHUNKS_IN_FLIGHT);
        let (cmd_tx, cmd_rx) = mpsc::channel::<RtlSdrCommand>();
        let ctrl = RtlSdrControl { tx: cmd_tx };
        thread::Builder::new()
            .name("RtlSdrSource-reader".to_string())
            .spawn(move || -> Result<()> {
                let mut dev =
                    rtlsdr::open(index).map_err(|e| Error::msg(format!("RTL SDR open: {e}")))?;
                debug!("Tuner type: {:?}", dev.get_tuner_type());
                dev.set_center_freq(freq as u32)?;
                debug!("Allowed tuner gains: {:?}", dev.get_tuner_gains()?);
                dev.set_tuner_gain(10 * igain)?;
                debug!("Tuner gain: {}", dev.get_tuner_gain());
                // dev.set_direct_sampling
                // dev.set_tuner_if_gain(…);
                // dev.set_tuner_gain_mode
                // dev.set_agc_mode
                dev.set_sample_rate(samp_rate)?;
                debug!("Set sample rate {}", dev.get_sample_rate()?);
                dev.reset_buffer()?;
                tx.send(vec![])?;
                loop {
                    // Apply any pending commands. We do this between reads so we don't
                    // need to interrupt the blocking `read_sync` call.
                    loop {
                        match cmd_rx.try_recv() {
                            Ok(RtlSdrCommand::CenterFreqHz(hz)) => {
                                if let Err(e) = dev.set_center_freq(hz) {
                                    warn!("RTL-SDR set_center_freq failed: {e}");
                                }
                            }
                            Ok(RtlSdrCommand::TunerGain(gain)) => {
                                if let Err(e) = dev.set_tuner_gain(10 * gain) {
                                    warn!("RTL-SDR set_tuner_gain failed: {e}");
                                }
                            }
                            Ok(RtlSdrCommand::SampleRate(sr)) => {
                                if let Err(e) = dev.set_sample_rate(sr) {
                                    warn!("RTL-SDR set_sample_rate failed: {e}");
                                } else if let Err(e) = dev.reset_buffer() {
                                    warn!("RTL-SDR reset_buffer after set_sample_rate failed: {e}");
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => break,
                        }
                    }

                    let buf = dev.read_sync(CHUNK_SIZE)?;
                    if let Err(e) = tx.send(buf) {
                        warn!("Failed to send message from RTL-SDR read thread to the block");
                        warn!("Assuming it's shutdown time");
                        return Err(e.into());
                    }
                }
            })?;
        assert_eq!(rx.recv()?, Vec::<u8>::new());
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            Self {
                rx,
                dst,
                buf: Vec::new(),
                control_tx: ctrl,
            },
            dr,
        ))
    }

    /// Returns a control handle that can retune parameters while the source is running.
    pub fn control(&self) -> RtlSdrControl {
        self.control_tx.clone()
    }
}

impl Block for RtlSdrSource {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            if !self.buf.is_empty() {
                let n = std::cmp::min(o.len(), self.buf.len());
                o.fill_from_slice(&self.buf[..n]);
                self.buf.drain(0..n);
                o.produce(n, &[]);
                continue;
            }
            return match self.rx.try_recv() {
                Err(TryRecvError::Empty) => Ok(BlockRet::Pending),
                Err(other) => Err(other.into()),
                Ok(buf) => {
                    let n = std::cmp::min(o.len(), buf.len());
                    o.fill_from_slice(&buf[..n]);
                    self.buf.extend(&buf[n..]);
                    o.produce(n, &[]);
                    continue;
                }
            };
        }
    }
}
