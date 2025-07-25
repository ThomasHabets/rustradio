/*! Hilbert transform.

[Wikipedia][wiki] has a bunch of math, but one use case for it is to
convert floating point values (think audio waveform) into upper
sideband.

Then again I guess you can do the same with a FloatToComplex plus
FftFilter.

This implementation is a pretty inefficient.

[wiki]: https://en.wikipedia.org/wiki/Hilbert_transform
*/

use crate::block::{Block, BlockRet};
use crate::fir::Fir;
use crate::stream::{ReadStream, WriteStream};
use crate::window::WindowType;
use crate::{Complex, Float, Result};

/// Hilbert transformer block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct Hilbert {
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
    history: Vec<Float>,
    filter: Fir<Float>,
    ntaps: usize,
}

impl Hilbert {
    /// Create new hilber transformer with this many taps.
    #[must_use]
    pub fn new(
        src: ReadStream<Float>,
        ntaps: usize,
        window_type: &WindowType,
    ) -> (Self, ReadStream<Complex>) {
        // TODO: take window function.
        assert!(ntaps & 1 == 1, "hilbert filter len must be odd");
        let taps = crate::fir::hilbert(&window_type.make_window(ntaps));
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                ntaps,
                dst,
                history: vec![0.0; ntaps],
                filter: Fir::new(&taps),
            },
            dr,
        )
    }
}

impl Block for Hilbert {
    fn work(&mut self) -> Result<BlockRet> {
        debug_assert_eq!(self.ntaps, self.history.len());
        let (ii, tags) = self.src.read_buf()?;
        let i = ii.slice();
        if i.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        let mut oo = self.dst.write_buf()?;
        let o = oo.slice();
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }

        let inout = std::cmp::min(i.len(), o.len());
        let len = self.history.len() + inout;
        let n = len - self.ntaps;

        if n == 0 {
            // TODO: use the right number instead of 1.
            return Ok(BlockRet::WaitForStream(
                if i.len() < o.len() {
                    &self.src
                } else {
                    &self.dst
                },
                1,
            ));
        }

        // TODO: Probably needless copy.
        let mut iv = Vec::with_capacity(len);
        iv.extend(&self.history);
        iv.extend(i.iter().take(inout).copied());

        use rayon::prelude::*;
        o.par_iter_mut().take(n).enumerate().for_each(|(i, val)| {
            let t = &iv[i..(i + self.ntaps)];
            *val = Complex::new(iv[i + self.ntaps / 2], self.filter.filter_float(t));
        });

        oo.produce(n, &tags);

        self.history[..self.ntaps].clone_from_slice(&iv[n..len]);
        ii.consume(n);
        Ok(BlockRet::Again)
    }
}
