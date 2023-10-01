use std::sync::mpsc;
use std::sync::mpsc::{RecvError, SendError, TryRecvError};
use std::thread;

use anyhow::Result;
use log::debug;

use crate::block::{get_output, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams};
use crate::Error;

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

#[cfg(test)]
mod tests {}

pub struct RtlSdrSource {
    rx: mpsc::Receiver<Vec<u8>>,
}

impl RtlSdrSource {
    pub fn new(freq: u64, samp_rate: u32, igain: i32) -> Result<Self, Error> {
        let index = 0;
        let found = rtlsdr::get_device_count();
        if index >= found {
            return Err(Error::new(&format!(
                "RTL SDR index {} doesn't exist, found {}",
                index, found
            )));
        }

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || -> Result<(), Error> {
            let mut dev =
                rtlsdr::open(index).map_err(|e| Error::new(&format!("RTL SDR open: {e}")))?;
            debug!("Tuner type: {:?}", dev.get_tuner_type());
            dev.set_center_freq(freq as u32)?;
            debug!("Allowed tuner gains: {:?}", dev.get_tuner_gains()?);
            dev.set_tuner_gain(igain)?;
            debug!("Tuner gain: {}", dev.get_tuner_gain());
            // dev.set_direct_sampling
            // dev.set_tuner_if_gain(…);
            // dev.set_tuner_gain_mode
            // dev.set_agc_mode
            dev.set_sample_rate(samp_rate)?;
            dev.reset_buffer()?;
            tx.send(vec![])?;
            loop {
                let chunk_size = 8192;
                let buf = dev.read_sync(chunk_size)?;
                tx.send(buf).unwrap();
            }
        });
        assert_eq!(rx.recv()?, vec![]);
        Ok(Self { rx })
    }
}

impl Block for RtlSdrSource {
    fn block_name(&self) -> &'static str {
        "RtlSdrSource"
    }
    fn work(&mut self, _r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let buf = match self.rx.try_recv() {
            Err(TryRecvError::Empty) => return Ok(BlockRet::Ok),
            Ok(x) => x,
            Err(other) => return Err(other.into()),
        };
        get_output(w, 0).borrow_mut().write_slice(&buf);
        Ok(BlockRet::Ok)
    }
}
