//! Decode RTL-SDR's byte based format into Complex I/Q.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
use crate::{Complex, Error, Float};

/// Decode RTL-SDR's byte based format into Complex I/Q.
pub struct RtlSdrDecode {
    src: Streamp<u8>,
    dst: Streamp<Complex>,
}

impl RtlSdrDecode {
    /// Create new RTL SDR Decode block.
    pub fn new(src: Streamp<u8>) -> Self {
        Self {
            src,
            dst: new_streamp(),
        }
    }
    /// Return the output stream.
    pub fn out(&self) -> Streamp<Complex> {
        self.dst.clone()
    }
}

impl Block for RtlSdrDecode {
    fn block_name(&self) -> &'static str {
        "RtlSdrDecode"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock().unwrap();
        let samples = input.available() - input.available() % 2;
        if samples == 0 {
            return Ok(BlockRet::Noop);
        }
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
