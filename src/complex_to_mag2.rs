//! Convert Complex numbers to square of their magnitude.
use anyhow::Result;

use crate::stream::{Stream, Streamp};
use crate::{map_block_convert_macro, Complex, Float};

/// Convert Complex numbers to square of their magnitude.
pub struct ComplexToMag2 {
    src: Streamp<Complex>,
    dst: Streamp<Float>,
}

impl ComplexToMag2 {
    /// Create new ComplexToMag2 block.
    pub fn new(src: Streamp<Complex>) -> Self {
        Self {
            src,
            dst: Stream::newp(),
        }
    }
    fn process_one(&self, sample: Complex) -> Float {
        sample.norm_sqr()
    }
}

map_block_convert_macro![ComplexToMag2, Float];
