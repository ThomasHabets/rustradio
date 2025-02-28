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
use log::{debug, warn};

use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};

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
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct RtlSdrSource {
    rx: mpsc::Receiver<Vec<u8>>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    buf: Vec<u8>,
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
    pub fn new(freq: u64, samp_rate: u32, igain: i32) -> Result<(Self, ReadStream<u8>), Error> {
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
            },
            dr,
        ))
    }
}

impl Block for RtlSdrSource {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        if !self.buf.is_empty() {
            let n = std::cmp::min(o.len(), self.buf.len());
            o.fill_from_slice(&self.buf[..n]);
            self.buf.drain(0..n);
            o.produce(n, &[]);
            return Ok(BlockRet::Ok);
        }
        match self.rx.try_recv() {
            Err(TryRecvError::Empty) => Ok(BlockRet::Pending),
            Err(other) => Err(other.into()),
            Ok(buf) => {
                let n = std::cmp::min(o.len(), buf.len());
                o.fill_from_slice(&buf[..n]);
                self.buf.extend(&buf[n..]);
                o.produce(n, &[]);
                Ok(BlockRet::Ok)
            }
        }
    }
}
