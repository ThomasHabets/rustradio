/*! Hilbert transform.

[Wikipedia][wiki] has a bunch of math, but one use case for it is to
convert floating point values (think audio waveform) into upper
sideband.

Then again I guess you can do the same with a FloatToComplex plus
FftFilter.

This implementation is a pretty inefficient.

[wiki]: https://en.wikipedia.org/wiki/Hilbert_transform
*/

use std::collections::VecDeque;

use crate::block::{Block, BlockRet};
use crate::fir::FIR;
use crate::stream::{new_streamp, Streamp};
use crate::{Complex, Error, Float};

trait IndexLen: std::ops::Index<usize, Output = Float> {
    fn len(&self) -> usize;
    fn extend_into(&self, v: &mut Vec<Float>);
}

impl IndexLen for Vec<Float> {
    fn len(&self) -> usize {
        Vec::<Float>::len(self)
    }
    fn extend_into(&self, v: &mut Vec<Float>) {
        v.extend(self);
    }
}
impl IndexLen for VecDeque<Float> {
    fn len(&self) -> usize {
        VecDeque::<Float>::len(self)
    }
    fn extend_into(&self, v: &mut Vec<Float>) {
        v.extend(self);
    }
}

struct StackedVec<'a> {
    vecs: Vec<&'a dyn IndexLen>,
}

impl<'a> StackedVec<'a> {
    fn new() -> Self {
        Self { vecs: Vec::new() }
    }
    fn len(&self) -> usize {
        self.vecs.iter().map(|x| x.len()).sum()
    }
    fn collect(&self) -> Vec<Float> {
        let mut t = Vec::with_capacity(self.len());
        for v in &self.vecs {
            v.extend_into(&mut t);
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stack_one() {
        let v = vec![0.1, 1.0, 2.0];
        let mut stack = StackedVec::new();
        stack.vecs.push(&v);
        assert_eq!(stack[0], 0.1);
    }
}

impl<'a> std::ops::Index<usize> for StackedVec<'a> {
    type Output = Float;
    fn index(&self, n: usize) -> &Float {
        let mut n = n;
        for cont in &self.vecs {
            if n < cont.len() {
                return &cont[n];
            }
            n -= cont.len();
        }
        panic!("Failed to index into stacked");
    }
}

/// Hilbert transformer block.
pub struct Hilbert {
    src: Streamp<Float>,
    dst: Streamp<Complex>,
    history: Vec<Float>,
    filter: FIR<Float>,
    ntaps: usize,
}

impl Hilbert {
    /// Create new hilber transformer with this many taps.
    pub fn new(src: Streamp<Float>, ntaps: usize) -> Self {
        assert!(ntaps & 1 == 1, "hilbert filter len must be odd");
        let taps = crate::fir::hilbert(ntaps); // TODO: provide window function.
        Self {
            src,
            ntaps,
            dst: new_streamp(),
            history: vec![0.0; ntaps],
            filter: FIR::new(&taps),
        }
    }
    /// Get the output stream.
    pub fn out(&self) -> Streamp<Complex> {
        self.dst.clone()
    }
}

impl Block for Hilbert {
    fn block_name(&self) -> &'static str {
        "Hilbert"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        assert_eq!(self.ntaps, self.history.len());
        let mut i = self.src.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut stack = StackedVec::new();
        stack.vecs.push(&self.history);
        stack.vecs.push(i.data());

        let len = stack.len();
        let mut v = Vec::with_capacity(len);

        // TODO: remove copy.
        let iv = stack.collect();

        for i in 0..(len - self.ntaps) {
            let t = &iv[i..(i + self.ntaps)];
            v.push(Complex::new(iv[i + self.ntaps / 2], self.filter.filter(t)));
        }
        self.dst.lock()?.write(v.iter().copied());

        // TODO: use fancy chained iterator.
        let mut newhist = Vec::with_capacity(self.ntaps);
        for i in (len - self.ntaps)..len {
            //self.history.extend(stack.iter().skip(len-self.ntaps));
            newhist.push(stack[i]);
        }
        self.history = newhist;
        i.clear();
        Ok(BlockRet::Ok)
    }
}
