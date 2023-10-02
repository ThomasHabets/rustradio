use anyhow::Result;

use crate::{map_block_convert_macro, Complex, Float};

pub struct QuadratureDemod {
    gain: Float,
    last: Complex,
}

impl QuadratureDemod {
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
