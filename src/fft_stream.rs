//! FFT stream.

use anyhow::Result;
use rustfft::FftPlanner;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Float};

#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FftStream {
    size: usize,
    fft: std::sync::Arc<dyn rustfft::Fft<Float>>,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl FftStream {
    pub fn new(src: ReadStream<Complex>, size: usize) -> (Self, ReadStream<Complex>) {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                size,
                fft,
                src,
                dst,
            },
            dr,
        )
    }
}

impl Block for FftStream {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (input, _tags) = self.src.read_buf()?;
        let ii = input.slice();
        if ii.len() < self.size {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        let oo = o.slice();
        if oo.len() < self.size {
            return Ok(BlockRet::Ok);
        }
        oo[..self.size].copy_from_slice(&ii[..self.size]);
        self.fft.process(oo);
        input.consume(self.size);
        o.produce(self.size, &[]);
        Ok(BlockRet::Ok)
    }
}
