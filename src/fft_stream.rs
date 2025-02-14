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
    threaded: bool,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl FftStream {
    /// Create a new FftStream.
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
                threaded: false,
            },
            dr,
        )
    }
    /// Turn on or off Rayon multithreading.
    ///
    /// Initial benchmarks seem to indicate that this does not help. Maybe with
    /// bigger than default stream buffers for more concurrency.
    pub fn threaded(&mut self, onoff: bool) {
        self.threaded = onoff;
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
        let len = std::cmp::min(ii.len(), oo.len());
        let len = len - (len % self.size);
        oo[..len].copy_from_slice(&ii[..len]);

        // It would be nice to use fft.process_outofplace_with_scratch(), but it
        // requires input also be scratch space, and therefore mutable.
        if self.threaded {
            use rayon::prelude::*;
            oo.par_chunks_exact_mut(self.size).for_each(|chunk| {
                self.fft.process(chunk);
            });
        } else {
            oo.chunks_exact_mut(self.size).for_each(|chunk| {
                self.fft.process(chunk);
            });
        }
        input.consume(len);
        o.produce(len, &[]);
        Ok(BlockRet::Ok)
    }
}
/* vim: textwidth=80
 */
