/*! Quadrature demod, the core of an FM demodulator.

Quadrature demodulation works is best done by thinking of the samples
as vectors going out of the origin on the complex plane.

A zero frequency means no "spinning" around the origin, but with all
samples just being on a vector, with the same angle, though possibly
varying magnitude.

Negative frequency means the vector is spinning counter
clockwise. Positive frequency means spinning clockwise.

Quadrature demodulation discards the magnitude of the vector, and just
looks at the angle between the current sample, and the previous
sample.

Because magnitude is discarded, this block is only useful for decoding
frequency changes (FM, FSK, …), not things like QAM.

[This article][vectorized] gives some good illustrations.

Enabling the `fast-math` feature (dependency) speeds up
QuadratureDemod by about 4x.

[vectorized]: https://mazzo.li/posts/vectorized-atan2.html
 */
use anyhow::Result;

use crate::stream::{new_streamp, Streamp};
use crate::{map_block_convert_macro, Complex, Float};

/// Quadrature demod, the core of an FM demodulator.
pub struct QuadratureDemod {
    gain: Float,
    last: Complex,
    src: Streamp<Complex>,
    dst: Streamp<Float>,
}

impl QuadratureDemod {
    /// Create new QuadratureDemod block.
    ///
    /// Gain is just used to scale the value, and can be set to 1.0 if
    /// you don't care about the scale.
    pub fn new(src: Streamp<Complex>, gain: Float) -> Self {
        Self {
            src,
            dst: new_streamp(),
            gain,
            last: Complex::default(),
        }
    }
    fn process_one(&mut self, s: Complex) -> Float {
        let t = s * self.last.conj();
        self.last = s;

        #[cfg(feature = "fast-math")]
        return self.gain * fast_math::atan2(t.im, t.re);

        #[cfg(not(feature = "fast-math"))]
        return self.gain * t.im.atan2(t.re);
    }
}
map_block_convert_macro![QuadratureDemod, Float];

/// A faster version of FM demodulation, that makes some assumptions.
///
/// This block can be used instead of a QuadratureDemod block, for
/// performance. It's much faster (~4x compared to the fast-math
/// version of QuadratureDemod), but it's less good.
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
/// enough. At 50ksps QuadratureDemod works well, but FastFM does
/// not. At 500ksps FastFM performs just as well in my tests. But
/// FastFM at 500ksps is about half the speed of QuadratureDemod at
/// 50ksps.
///
/// Really, just use QuadratureDemod unless it's shown to be too slow
/// for your use case.
///
/// Lyons has an more general version of this algorithm, also on page
/// 760, but it's not implemented here.
pub struct FastFM {
    src: Streamp<Complex>,
    dst: Streamp<Float>,
    q1: Complex,
    q2: Complex,
}

impl FastFM {
    /// Create a new FastFM block.
    pub fn new(src: Streamp<Complex>) -> Self {
        Self {
            src,
            dst: new_streamp(),
            q1: Complex::default(),
            q2: Complex::default(),
        }
    }

    fn process_one(&mut self, s: Complex) -> Float {
        let top = (s.im - self.q2.im) * self.q1.re;
        let bottom = (s.re - self.q2.re) * self.q1.im;
        self.q2 = self.q1;
        self.q1 = s;
        top - bottom
    }
}
map_block_convert_macro![FastFM, Float];
