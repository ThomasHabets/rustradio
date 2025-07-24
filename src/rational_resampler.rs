//! Resample by a fractional amount.
/*
* Unlike the rational resampler in GNURadio, this one doesn't filter.
 */
use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    a
}

pub struct RationalResamplerBuilder<T> {
    dummy: std::marker::PhantomData<T>,
}
pub struct RationalResamplerBuilderInterp<T> {
    dummy: std::marker::PhantomData<T>,
    interp: usize,
}
pub struct RationalResamplerBuilderDeci<T> {
    dummy: std::marker::PhantomData<T>,
    deci: usize,
}
pub struct RationalResamplerBuilderBoth<T> {
    dummy: std::marker::PhantomData<T>,
    interp: usize,
    deci: usize,
}

impl<T> Default for RationalResamplerBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> RationalResamplerBuilder<T> {
    pub fn new() -> Self {
        Self {
            dummy: std::marker::PhantomData,
        }
    }
    pub fn deci(self, deci: usize) -> RationalResamplerBuilderDeci<T> {
        RationalResamplerBuilderDeci {
            deci,
            dummy: self.dummy,
        }
    }
    pub fn interp(self, interp: usize) -> RationalResamplerBuilderInterp<T> {
        RationalResamplerBuilderInterp {
            interp,
            dummy: self.dummy,
        }
    }
}
impl<T> RationalResamplerBuilderInterp<T> {
    pub fn deci(self, deci: usize) -> RationalResamplerBuilderBoth<T> {
        RationalResamplerBuilderBoth {
            interp: self.interp,
            deci,
            dummy: self.dummy,
        }
    }
}
impl<T> RationalResamplerBuilderDeci<T> {
    #[must_use]
    pub fn interp(self, interp: usize) -> RationalResamplerBuilderBoth<T> {
        RationalResamplerBuilderBoth {
            deci: self.deci,
            interp,
            dummy: self.dummy,
        }
    }
}
impl<T: Sample> RationalResamplerBuilderBoth<T> {
    pub fn build(self, src: ReadStream<T>) -> Result<(RationalResampler<T>, ReadStream<T>)> {
        RationalResampler::new(src, self.interp, self.deci)
    }
}

/// Resample by a fractional amount.
///
/// This can be used to easily convert from any sample rate to any other. Just
/// set decimation to the current rate, and interpolation to the new rate.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct RationalResampler<T: Sample> {
    deci: i64,
    interp: i64,
    counter: i64,

    #[rustradio(in)]
    src: ReadStream<T>,

    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample> RationalResampler<T> {
    /// Create builder.
    pub fn builder() -> RationalResamplerBuilder<T> {
        RationalResamplerBuilder::<T>::new()
    }

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

impl<T: Sample> Block for RationalResampler<T> {
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: retain tags.
        let (i, _tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }

        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let mut opos = 0;
        let mut taken = 0;
        let mut out_full = false;
        'outer: for s in i.iter() {
            taken += 1;
            self.counter += self.interp;
            while self.counter > 0 {
                o.slice()[opos] = *s;
                self.counter -= self.deci;
                opos += 1;
                if opos == o.len() {
                    out_full = true;
                    break 'outer;
                }
            }
        }
        i.consume(taken);
        o.produce(opos, &[]);
        Ok(if out_full {
            BlockRet::WaitForStream(&self.dst, 1)
        } else {
            BlockRet::WaitForStream(&self.src, 1)
        })
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::tests::assert_almost_equal_complex;
    use crate::{Complex, Float};

    #[test]
    fn deci() -> Result<()> {
        let input = vec![
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.2),
            Complex::new(4.1, 0.0),
            Complex::new(5.0, 0.0),
            Complex::new(6.0, 0.2),
        ];
        for deci in 1..=(input.len() + 1) {
            let (mut src, src_out) = VectorSource::new(input.clone());
            assert!(matches![src.work()?, BlockRet::EOF]);
            let (mut b, os) = RationalResampler::new(src_out, 1, deci)?;
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
            let (res, _) = os.read_buf()?;
            // TODO: test tags
            assert_almost_equal_complex(
                res.slice(),
                &input.iter().copied().step_by(deci).collect::<Vec<_>>(),
            );
        }
        Ok(())
    }

    #[test]
    fn example64() -> Result<()> {
        let input: Vec<_> = (0..50).collect();
        let (mut src, src_out) = VectorSource::new(input.clone());
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut b, os) = RationalResampler::new(src_out, 25, 64)?;
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
        let (res, _) = os.read_buf()?;
        // TODO: test tags
        assert_eq!(
            res.slice(),
            &[
                0, 2, 5, 7, 10, 12, 15, 17, 20, 23, 25, 28, 30, 33, 35, 38, 40, 43, 46, 48
            ],
        );
        Ok(())
    }

    #[test]
    fn example128() -> Result<()> {
        let input: Vec<_> = (0..50).collect();
        let (mut src, src_out) = VectorSource::new(input.clone());
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut b, os) = RationalResampler::new(src_out, 25, 128)?;
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
        let (res, _) = os.read_buf()?;
        assert_eq!(res.slice(), &[0, 5, 10, 15, 20, 25, 30, 35, 40, 46]);
        Ok(())
    }

    #[test]
    fn chained() -> Result<()> {
        let input: Vec<_> = (0..5000).collect();

        // Path 1: direct 25/128.
        let path1 = {
            let (mut src, src_out) = VectorSource::new(input.clone());
            assert!(matches![src.work()?, BlockRet::EOF]);

            let (mut b, os) = RationalResampler::new(src_out, 25, 128)?;
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
            os.read_buf()?.0
        };

        // Path 2: first 1/2, then 25/64.
        let path2 = {
            let (mut src, src_out) = VectorSource::new(input.clone());
            assert!(matches![src.work()?, BlockRet::EOF]);

            let (mut b1, os1) = RationalResampler::new(src_out, 1, 2)?;
            assert!(matches![b1.work()?, BlockRet::WaitForStream(_, _)]);

            let (mut b2, os) = RationalResampler::new(os1, 25, 64)?;
            assert!(matches![b2.work()?, BlockRet::WaitForStream(_, _)]);
            os.read_buf()?.0
        };
        assert_eq!(path1.len(), path2.len());
        path1
            .iter()
            .zip(path2.iter())
            .enumerate()
            .for_each(|(n, (&a, &b))| {
                let abs = if a > b { a - b } else { b - a };
                assert!(
                    abs < 2,
                    "mismatch as position {n}: diff of more than 1 for {a} vs {b}"
                );
            });
        //assert_eq!(res.slice(), &[0, 5, 10, 15, 20, 25, 30, 35, 40, 46]);
        Ok(())
    }

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
            res.len(),
            res.slice()
        );
        Ok(())
    }

    #[test]
    fn rates() -> Result<()> {
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
