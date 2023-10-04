//! Resample by a fractional amount.
/*
* Unlike the rational resampler in GNURadio, this one doesn't filter.
*/
use anyhow::Result;

use crate::block::{get_input, get_output, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams};
use crate::{Complex, Error, Float};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{StreamType, Streamp};
    use crate::Float;

    fn runtest(inputsize: usize, interp: usize, deci: usize, finalcount: usize) -> Result<()> {
        let input: Vec<_> = (0..inputsize)
            .map(|i| Complex::new(i as Float, 0.0))
            .collect();
        let mut is = InputStreams::new();
        let mut os = OutputStreams::new();
        is.add_stream(StreamType::new_complex_from_slice(&input));
        os.add_stream(StreamType::new_complex());
        let mut resamp = RationalResampler::new(interp, deci)?;
        resamp.work(&mut is, &mut os)?;
        let res: Streamp<Complex> = os.get(0).into();
        assert_eq!(
            finalcount,
            res.borrow().available(),
            "{:?}",
            res.borrow().data()
        );
        Ok(())
    }

    #[test]
    fn foo() -> Result<()> {
        runtest(10, 1, 1, 10)?;
        runtest(10, 1, 2, 5)?;
        runtest(10, 2, 1, 20)?;
        runtest(100, 2, 3, 66)?;
        runtest(100, 3, 2, 150)?;
        runtest(100, 300, 200, 150)?;
        runtest(100, 200000, 1024000, 19)?;
        Ok(())
    }
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    a
}

/// Resample by a fractional amount.
pub struct RationalResampler {
    deci: i64,
    interp: i64,
    counter: i64,
}

impl RationalResampler {
    /// Create new RationalResampler block.
    ///
    /// A common pattern to convert between arbitrary sample rates X
    /// and Y is to decimate by X and interpolate by Y.
    pub fn new(mut interp: usize, mut deci: usize) -> Result<Self> {
        let g = gcd(deci, interp);
        deci /= g;
        interp /= g;
        Ok(Self {
            interp: i64::try_from(interp)?,
            deci: i64::try_from(deci)?,
            counter: 0,
        })
    }
}

impl Block for RationalResampler {
    fn block_name(&self) -> &'static str {
        "RationalResampler"
    }
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        if r.is_complex(0) {
            let mut v = Vec::new();
            self.counter -= self.deci;
            for s in get_input(r, 0).borrow().iter() {
                self.counter += self.interp;
                while self.counter >= 0 {
                    v.push(*s);
                    self.counter -= self.deci;
                }
            }
            get_input::<Complex>(r, 0).borrow_mut().clear();
            get_output::<Complex>(w, 0).borrow_mut().write_slice(&v);
        } else if r.is_float(0) {
            let mut v = Vec::new();
            self.counter -= self.deci;
            for s in get_input(r, 0).borrow().iter() {
                self.counter += self.interp;
                while self.counter >= 0 {
                    v.push(*s);
                    self.counter -= self.deci;
                }
            }
            get_input::<Float>(r, 0).borrow_mut().clear();
            get_output::<Float>(w, 0).borrow_mut().write_slice(&v);
        } else {
            panic!("Other types not allowed");
        }
        Ok(BlockRet::Ok)
    }
}
