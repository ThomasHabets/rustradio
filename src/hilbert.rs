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
use crate::fir::FIR;
use crate::stream::{Stream, Streamp};
use crate::window::WindowType;
use crate::{Complex, Error, Float};

/// Hilbert transformer block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, out)]
pub struct Hilbert {
    #[rustradio(in)]
    src: Streamp<Float>,
    #[rustradio(out)]
    dst: Streamp<Complex>,
    history: Vec<Float>,
    filter: FIR<Float>,
    ntaps: usize,
}

impl Hilbert {
    /// Create new hilber transformer with this many taps.
    pub fn new(src: Streamp<Float>, ntaps: usize, window_type: &WindowType) -> Self {
        // TODO: take window function.
        assert!(ntaps & 1 == 1, "hilbert filter len must be odd");
        let taps = crate::fir::hilbert(&window_type.make_window(ntaps));
        Self {
            src,
            ntaps,
            dst: Stream::newp(),
            history: vec![0.0; ntaps],
            filter: FIR::new(&taps),
        }
    }
}

impl Block for Hilbert {
    fn work(&mut self) -> Result<BlockRet, Error> {
        assert_eq!(self.ntaps, self.history.len());
        let (i, tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::OutputFull);
        }

        let inout = std::cmp::min(i.len(), o.len());
        let len = self.history.len() + inout;
        let n = len - self.ntaps;

        // TODO: Probably needless copy.
        let mut iv = Vec::with_capacity(len);
        iv.extend(&self.history);
        iv.extend(i.iter().take(inout).copied());

        // I tried a couple of variations of this loop, and this was
        // the fastest on my laptop.
        for i in 0..n {
            let t = &iv[i..(i + self.ntaps)];
            o.slice()[i] = Complex::new(iv[i + self.ntaps / 2], self.filter.filter(t));
        }

        o.produce(n, &tags);

        self.history[..self.ntaps].clone_from_slice(&iv[n..len]);
        i.consume(n);
        Ok(BlockRet::Ok)
    }
}
