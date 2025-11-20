//! Quadrature demod, the core of an FM demodulator.

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float, Result};

/// Quadrature demod, the core of an FM demodulator.
///
/// Quadrature demodulation works is best done by thinking of the samples
/// as vectors going out of the origin on the complex plane.
///
/// A zero frequency means no "spinning" around the origin, but with all
/// samples just being on a vector, with the same angle, though possibly
/// varying magnitude.
///
/// Negative frequency means the vector is spinning counter
/// clockwise. Positive frequency means spinning clockwise.
///
/// Quadrature demodulation discards the magnitude of the vector, and just
/// looks at the angle between the current sample, and the previous
/// sample.
///
/// Because magnitude is discarded, this block is only useful for decoding
/// frequency changes (FM, FSK, …), not things like QAM.
///
/// [This article][vectorized] gives some good illustrations.
///
/// Enabling the `fast-math` feature (dependency) speeds up
/// `QuadratureDemod` by about 4x.
///
/// [vectorized]: https://mazzo.li/posts/vectorized-atan2.html
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct QuadratureDemod {
    gain: Float,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Float>,

    #[rustradio(default)]
    tmp: Vec<Complex>,
}

impl Block for QuadratureDemod {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (inp, _) = self.src.read_buf()?;
        if inp.len() < 2 {
            return Ok(BlockRet::WaitForStream(&self.src, 2));
        }
        let mut out = self.dst.write_buf()?;
        if out.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let n = inp.len().min(out.len());
        let n1 = n - 1;
        let o = &mut out.slice()[..n1];
        let i = &inp.slice()[..n];
        if self.tmp.len() < n {
            self.tmp.resize(n, Complex::default());
        }

        // Conjugate.

        #[cfg(feature = "volk")]
        volk::volk_32fc_x2_multiply_conjugate_32fc(&mut self.tmp[..n1], &i[1..n], &i[..n1]);
        #[cfg(not(feature = "volk"))]
        {
            for t in 0..n1 {
                self.tmp[t] = i[t].conj() * i[t + 1];
            }
        }

        // atan2
        #[cfg(feature = "fast-math")]
        {
            for (i, item) in o.iter_mut().enumerate().take(n1) {
                *item = self.gain * fast_math::atan2(self.tmp[i].im, self.tmp[i].re);
            }
            if false {
                // Maybe this can be faster in some circumstances, but not yet in my
                // testing.
                use rayon::iter::IndexedParallelIterator;
                use rayon::iter::IntoParallelRefIterator;
                use rayon::iter::IntoParallelRefMutIterator;
                use rayon::iter::ParallelIterator;
                o.par_iter_mut()
                    .zip(self.tmp.par_iter())
                    .for_each(|(a, b)| {
                        *a = self.gain * fast_math::atan2(b.im, b.re);
                    });
            }
        }
        #[cfg(not(feature = "fast-math"))]
        {
            // This is way slower than fast-math. Fast-math atan2 is just that
            // fast. Maybe one day it'll be in volk.
            //
            // https://mazzo.li/posts/vectorized-atan2.html
            #[cfg(feature = "volk")]
            volk::volk_32fc_s32f_atan2_32f(&mut out.slice()[..n1], &self.tmp[..n1], self.gain);

            #[cfg(not(feature = "volk"))]
            o.iter_mut().zip(self.tmp.iter()).for_each(|(a, b)| {
                *a = self.gain * b.im.atan2(b.re);
            });
        }
        inp.consume(n1);
        out.produce(n1, &[]);
        Ok(BlockRet::Pending)
    }
}

/// A faster version of FM demodulation, that makes some assumptions.
///
/// This block can be used instead of a `QuadratureDemod` block, for
/// performance. It's much faster (~4x compared to the fast-math
/// version of `QuadratureDemod`), but it's less good.
///
/// The algorithm is taken from Lyons, Understanding Digital Signal
/// Processing, third edition, page 760.
///
/// This is the faster version of the two, which assumes all
/// frequencies are constant amplitude. This means it can be used to
/// e.g. demodulate an FM carrier for 1200bps AX.25, but *not* to
/// decode the preemphasized bell 202 inside.
///
/// You could deemphasize, if you know all transmitters preemp
/// parameters.
///
/// For 9600bps AX.25 it works fine, if the sample rate is high
/// enough. At 50ksps `QuadratureDemod` works well, but `FastFM` does
/// not. At 500ksps `FastFM` performs just as well in my tests. But
/// `FastFM` at 500ksps is about half the speed of `QuadratureDemod` at
/// 50ksps.
///
/// Really, just use `QuadratureDemod` unless it's shown to be too slow
/// for your use case.
///
/// Lyons has an more general version of this algorithm, also on page
/// 760, but it's not implemented here.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct FastFM {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
    #[rustradio(default)]
    q1: Complex,
    #[rustradio(default)]
    q2: Complex,
}

impl FastFM {
    fn process_sync(&mut self, s: Complex) -> Float {
        let top = (s.im - self.q2.im) * self.q1.re;
        let bottom = (s.re - self.q2.re) * self.q1.im;
        self.q2 = self.q1;
        self.q1 = s;
        top - bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::tests::assert_almost_equal_float;

    #[test]
    fn quad_nulls() -> Result<()> {
        let (mut b, prev) = VectorSource::new(vec![Complex::default(); 4]);
        b.work()?;
        let (mut b, out) = QuadratureDemod::new(prev, 1.0);
        b.work()?;
        let (o, _) = out.read_buf()?;
        assert_eq!(o.slice(), vec![0.0f32; 3]);
        Ok(())
    }

    #[test]
    fn quad_cw() -> Result<()> {
        let (mut b, prev) = VectorSource::new(vec![
            Complex::new(1.0, 0.0),
            Complex::new(0.707, -0.707),
            Complex::new(0.0, -1.0),
            Complex::new(-1.0, 0.0),
        ]);
        b.work()?;
        let (mut b, out) = QuadratureDemod::new(prev, 1.0);
        b.work()?;
        let (o, _) = out.read_buf()?;
        assert_almost_equal_float(
            o.slice(),
            &[
                -std::f32::consts::PI / 4.0,
                -std::f32::consts::PI / 4.0,
                -std::f32::consts::PI / 2.0,
            ],
        );
        Ok(())
    }
    #[test]
    fn quad_ccw() -> Result<()> {
        let (mut b, prev) = VectorSource::new(vec![
            Complex::new(1.0, 0.0),
            Complex::new(0.707, 0.707),
            Complex::new(0.0, 1.0),
            Complex::new(-1.0, 0.0),
        ]);
        b.work()?;
        let (mut b, out) = QuadratureDemod::new(prev, 1.0);
        b.work()?;
        let (o, _) = out.read_buf()?;
        assert_almost_equal_float(
            o.slice(),
            &[
                std::f32::consts::PI / 4.0,
                std::f32::consts::PI / 4.0,
                std::f32::consts::PI / 2.0,
            ],
        );
        Ok(())
    }
}
