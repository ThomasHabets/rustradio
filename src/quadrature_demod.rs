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
use std::sync::{Arc, Mutex};

use crate::stream::Stream;
use crate::{map_block_convert_macro, Complex, Float};

/// Quadrature demod, the core of an FM demodulator.
pub struct QuadratureDemod {
    gain: Float,
    last: Complex,
    src: Arc<Mutex<Stream<Complex>>>,
    dst: Arc<Mutex<Stream<Float>>>,
}

impl QuadratureDemod {
    /// Create new QuadratureDemod block.
    ///
    /// Gain is just used to scale the value, and can be set to 1.0 if
    /// you don't care about the scale.
    pub fn new(src: Arc<Mutex<Stream<Complex>>>, gain: Float) -> Self {
        Self {
            src,
            dst: Arc::new(Mutex::new(Stream::new())),
            gain,
            last: Complex::default(),
        }
    }
    pub fn out(&self) -> Arc<Mutex<Stream<Float>>> {
        self.dst.clone()
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
map_block_convert_macro![QuadratureDemod];
