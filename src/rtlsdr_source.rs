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
use std::sync::mpsc::{RecvError, SendError, TryRecvError};
use std::thread;

use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams};
use crate::Error;

const CHUNK_SIZE: usize = 8192;
const MAX_CHUNKS_IN_FLIGHT: usize = 1000;

impl From<rtlsdr::RTLSDRError> for Error {
    fn from(e: rtlsdr::RTLSDRError) -> Self {
        Error::new(&format!("RTL SDR Error: {}", e))
    }
}

impl From<RecvError> for Error {
    fn from(e: RecvError) -> Self {
        Error::new(&format!("recv error: {}", e))
    }
}
impl From<TryRecvError> for Error {
    fn from(e: TryRecvError) -> Self {
        Error::new(&format!("recv error: {}", e))
    }
}

impl<T> From<SendError<T>> for Error {
    fn from(e: SendError<T>) -> Self {
        Error::new(&format!("send error: {}", e))
    }
}

/// RTL SDR Source block.
pub struct RtlSdrSource {
    rx: mpsc::Receiver<Vec<u8>>,
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
    pub fn new(freq: u64, samp_rate: u32, igain: i32) -> Result<Self, Error> {
        let index = 0;
        let found = rtlsdr::get_device_count();
        if index >= found {
            return Err(Error::new(&format!(
                "RTL SDR index {} doesn't exist, found {}",
                index, found
            )));
        }

        let (tx, rx) = mpsc::sync_channel(MAX_CHUNKS_IN_FLIGHT);
        thread::Builder::new()
            .name("RtlSdrSource-reader".to_string())
            .spawn(move || -> Result<(), Error> {
                let mut dev =
                    rtlsdr::open(index).map_err(|e| Error::new(&format!("RTL SDR open: {e}")))?;
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
                    let buf = dev.read_sync(CHUNK_SIZE)?;
                    tx.send(buf)
                        .expect("Failed to send message from RTL-SDR read thread to the block");
                }
            })?;
        assert_eq!(rx.recv()?, vec![]);
        Ok(Self { rx })
    }
}

impl Block for RtlSdrSource {
    fn block_name(&self) -> &'static str {
        "RtlSdrSource"
    }
    fn work(&mut self, _r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        match self.rx.try_recv() {
            Err(TryRecvError::Empty) => Ok(BlockRet::Ok),
            Err(other) => Err(other.into()),
            Ok(buf) => {
                w.get(0).borrow_mut().write_slice(&buf);
                Ok(BlockRet::Ok)
            }
        }
    }
}
