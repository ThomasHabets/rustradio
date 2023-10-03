//! Convert Complex numbers to square of their magnitude.
use anyhow::Result;

use crate::{map_block_convert_macro, Complex, Float};

/// Convert Complex numbers to square of their magnitude.
pub struct ComplexToMag2;

impl ComplexToMag2 {
    /// Create new ComplexToMag2 block.
    pub fn new() -> Self {
        Self {}
    }
    fn process_one(&self, sample: Complex) -> Float {
        sample.norm_sqr()
    }
}

impl Default for ComplexToMag2 {
    fn default() -> Self {
        Self::new()
    }
}
map_block_convert_macro![ComplexToMag2];
