//! Decode RTL-SDR's byte based format into Complex I/Q.
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Stream;
use crate::{Complex, Error, Float};

/// Decode RTL-SDR's byte based format into Complex I/Q.
pub struct RtlSdrDecode {
    src: Arc<Mutex<Stream<u8>>>,
    dst: Arc<Mutex<Stream<Complex>>>,
}

impl RtlSdrDecode {
    /// Create new RTL SDR Decode block.
    pub fn new(src: Arc<Mutex<Stream<u8>>>) -> Self {
        Self {
            src,
            dst: Arc::new(Mutex::new(Stream::new())),
        }
    }
    pub fn out(&self) -> Arc<Mutex<Stream<Complex>>> {
        self.dst.clone()
    }
}

impl Block for RtlSdrDecode {
    fn block_name(&self) -> &'static str {
        "RtlSdrDecode"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock().unwrap();
        let samples: usize = input.available() - input.available() % 2;
        let mut out = self.dst.lock().unwrap();

        // TODO: needless copy.
        let buf: Vec<u8> = input.data().clone().into();
        let buf = &buf[..samples];
        out.write_slice(
            (0..samples)
                .step_by(2)
                .map(|e| {
                    Complex::new(
                        ((buf[e] as Float) - 127.0) * 0.008,
                        ((buf[e + 1] as Float) - 127.0) * 0.008,
                    )
                })
                .collect::<Vec<Complex>>()
                .as_slice(),
        );
        input.consume(samples);
        Ok(BlockRet::Ok)
    }
}
