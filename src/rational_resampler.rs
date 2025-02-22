//! Resample by a fractional amount.
/*
* Unlike the rational resampler in GNURadio, this one doesn't filter.
 */
use anyhow::Result;
use log::trace;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::Error;

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    a
}

/// Resample by a fractional amount.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct RationalResampler<T: Copy> {
    deci: i64,
    interp: i64,
    counter: i64,

    #[rustradio(in)]
    src: ReadStream<T>,

    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Copy> RationalResampler<T> {
    /// Create new RationalResampler block.
    ///
    /// A common pattern to convert between arbitrary sample rates X
    /// and Y is to decimate by X and interpolate by Y.
    pub fn new(
        src: ReadStream<T>,
        mut interp: usize,
        mut deci: usize,
    ) -> Result<(Self, ReadStream<T>)> {
        let g = gcd(deci, interp);
        deci /= g;
        interp /= g;
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            Self {
                interp: i64::try_from(interp)?,
                deci: i64::try_from(deci)?,
                counter: 0,
                src,
                dst,
            },
            dr,
        ))
    }
}

impl<T: Copy> Block for RationalResampler<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _tags) = self.src.read_buf()?;

        {
            let iwant = self.interp as usize + 1;
            if i.len() < iwant {
                return Ok(BlockRet::WaitForStream(&self.src, iwant));
            }
        }
        let mut o = self.dst.write_buf()?;
        {
            let owant = self.deci as usize + 1;
            if o.len() < owant {
                return Ok(BlockRet::WaitForStream(&self.dst, owant));
            }
        }
        let n = std::cmp::min(i.len() - self.interp as usize, o.len() - self.deci as usize);
        trace!("RationalResampler: n = {n}");
        assert_ne!(n, 0);
        let mut opos = 0;
        let mut taken = 0;
        'outer: for s in i.iter() {
            taken += 1;
            self.counter += self.interp;
            while self.counter > 0 {
                o.slice()[opos] = *s;
                self.counter -= self.deci;
                opos += 1;
                if opos == o.len() {
                    break 'outer;
                }
            }
        }
        i.consume(taken);
        o.produce(opos, &[]);
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::{Complex, Float};

    fn runtest(inputsize: usize, interp: usize, deci: usize, finalcount: usize) -> Result<()> {
        let input: Vec<_> = (0..inputsize)
            .map(|i| Complex::new(i as Float, 0.0))
            .collect();
        let (mut src, src_out) = VectorSource::new(input);
        src.work()?;
        let (mut resamp, os) = RationalResampler::new(src_out, interp, deci)?;
        resamp.work()?;
        let (res, _) = os.read_buf()?;
        assert_eq!(
            finalcount,
            res.len(),
            "inputsize={inputsize} interp={interp} deci={deci} finalcount={finalcount}: Actual={} values={:?}",
            res.len(), res.slice()
        );
        Ok(())
    }

    #[test]
    fn foo() -> Result<()> {
        runtest(10, 1, 1, 10)?;
        runtest(10, 1, 2, 5)?;
        runtest(10, 2, 1, 20)?;
        runtest(100, 2, 3, 67)?;
        runtest(100, 3, 2, 150)?;
        runtest(100, 300, 200, 150)?;
        runtest(100, 200000, 1024000, 20)?;
        Ok(())
    }
}
