//! Turn positive Float values into binary `1u8`, and negative into `0u8`.
use anyhow::Result;

use crate::{map_block_convert_macro, Float};

pub struct BinarySlicer;

impl BinarySlicer {
    pub fn new() -> Self {
        Self {}
    }

    fn process_one(&self, a: Float) -> u8 {
        if a > 0.0 {
            1
        } else {
            0
        }
    }
}

impl Default for BinarySlicer {
    fn default() -> Self {
        Self::new()
    }
}

map_block_convert_macro![BinarySlicer];
