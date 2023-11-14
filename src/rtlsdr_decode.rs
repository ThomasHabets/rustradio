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
        // TODO: handle tags.
        let (input, _tags) = self.src.read_buf()?;
        let isamples = input.len() - input.len() % 2;
        let osamples = isamples / 2;
        if isamples == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut out = self.dst.write_buf()?;

        // TODO: needless copy.
        out.slice()[..osamples].clone_from_slice(
            (0..isamples)
                .step_by(2)
                .map(|e| {
                    Complex::new(
                        ((input[e] as Float) - 127.0) * 0.008,
                        ((input[e + 1] as Float) - 127.0) * 0.008,
                    )
                })
                .collect::<Vec<Complex>>()
                .as_slice(),
        );
        input.consume(isamples);
        out.produce(osamples, &[]);
        Ok(BlockRet::Ok)
    }
}
