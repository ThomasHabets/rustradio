//! Decode RTL-SDR's byte based format into Complex I/Q.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::{Complex, Error, Float};

/// Decode RTL-SDR's byte based format into Complex I/Q.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out)]
pub struct RtlSdrDecode {
    #[rustradio(in)]
    src: Streamp<u8>,
    #[rustradio(out)]
    dst: Streamp<Complex>,
}

impl Block for RtlSdrDecode {
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
        out.fill_from_iter((0..isamples).step_by(2).map(|e| {
            Complex::new(
                ((input[e] as Float) - 127.0) * 0.008,
                ((input[e + 1] as Float) - 127.0) * 0.008,
            )
        }));
        input.consume(isamples);
        out.produce(osamples, &[]);
        Ok(BlockRet::Ok)
    }
}
