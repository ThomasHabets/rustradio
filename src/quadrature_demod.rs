//! Quadrature demod, the core of an FM demodulator.
use anyhow::Result;

use crate::{map_block_convert_macro, Complex, Float};

/// Quadrature demod, the core of an FM demodulator.
pub struct QuadratureDemod {
    gain: Float,
    last: Complex,
}

impl QuadratureDemod {
    /// Create new QuadratureDemod block.
    ///
    /// Gain is just used to scale the value, and can be set to 1.0 if
    /// you don't care about the scale.
    pub fn new(gain: Float) -> Self {
        Self {
            gain,
            last: Complex::default(),
        }
    }
    fn process_one(&mut self, s: Complex) -> Float {
        let t = s * self.last.conj();
        self.last = s;
        self.gain * t.im.atan2(t.re)
    }
}
map_block_convert_macro![QuadratureDemod];
