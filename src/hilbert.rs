/*! Hilbert transform.

[Wikipedia][wiki] has a bunch of math, but one use case for it is to
convert floating point values (think audio waveform) into upper
sideband.

Then again I guess you can do the same with a `FloatToComplex` plus
`FftFilter`.

This implementation is a pretty inefficient.

[wiki]: https://en.wikipedia.org/wiki/Hilbert_transform
*/

use crate::block::{Block, BlockRet};
use crate::fir::Fir;
use crate::stream::{ReadStream, WriteStream};
use crate::window::WindowType;
use crate::{Complex, Error, Float, Result};

/// Hilbert transformer block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct Hilbert {
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
    history: Box<[Float]>,
    filter: Fir<Float>,
    ntaps: usize,
    threaded: bool,
}

impl Hilbert {
    /// Create a Hilbert transformer with this many taps.
    ///
    /// Returns an error unless the tap count is odd and greater than one.
    pub fn new(
        src: ReadStream<Float>,
        ntaps: usize,
        window_type: &WindowType,
    ) -> Result<(Self, ReadStream<Complex>)> {
        if ntaps <= 1 || ntaps & 1 == 0 {
            return Err(Error::msg(
                "Hilbert tap count must be odd and greater than one",
            ));
        }
        let taps = crate::fir::hilbert(&window_type.make_window(ntaps));
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            Self {
                src,
                ntaps,
                dst,
                history: vec![0.0; ntaps].into_boxed_slice(),
                filter: Fir::new(taps)?,
                threaded: false,
            },
            dr,
        ))
    }
    /// Turn on or off Rayon multithreading.
    ///
    /// Initial benchmarks seem to indicate that this does not help. Maybe with
    /// bigger than default stream buffers for more concurrency.
    pub fn threaded(&mut self, onoff: bool) {
        self.threaded = onoff;
    }
}

impl Block for Hilbert {
    fn work(&mut self) -> Result<BlockRet<'_>> {
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

        let n = std::cmp::min(i.len(), o.len());
        let len = self.history.len() + n;

        // TODO: Probably needless copy.
        let mut iv = Vec::with_capacity(len);
        iv.extend(&self.history);
        iv.extend(i.iter().take(n).copied());

        if self.threaded {
            use rayon::prelude::*;
            o.par_iter_mut().take(n).enumerate().for_each(|(i, val)| {
                let t = &iv[i..(i + self.ntaps)];
                *val = Complex::new(iv[i + self.ntaps / 2], self.filter.filter_float(t));
            });
        } else {
            o.iter_mut().take(n).enumerate().for_each(|(i, val)| {
                let t = &iv[i..(i + self.ntaps)];
                *val = Complex::new(iv[i + self.ntaps / 2], self.filter.filter_float(t));
            });
        }

        let tags = tags
            .into_iter()
            .take_while(|tag| tag.pos() < n)
            .collect::<Vec<_>>();
        oo.produce(n, &tags);

        self.history.clone_from_slice(&iv[n..len]);
        ii.consume(n);
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_tap_counts() {
        for ntaps in [0, 1, 2, 4] {
            assert!(
                Hilbert::new(ReadStream::from_slice(&[0.0]), ntaps, &WindowType::Hamming).is_err()
            );
        }
    }

    #[test]
    fn valid_transform_outputs_one_sample_per_input() -> Result<()> {
        let (mut block, out) = Hilbert::new(
            ReadStream::from_slice(&[1.0, 2.0, 3.0]),
            3,
            &WindowType::Hamming,
        )?;
        assert!(matches!(block.work()?, BlockRet::Again));
        assert_eq!(out.read_buf()?.0.len(), 3);
        Ok(())
    }
}
