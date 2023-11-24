//! Turn positive Float values into binary `1u8`, and negative into `0u8`.
use anyhow::Result;

use crate::stream::{new_streamp, ReadStream, ReadStreamp, Streamp};
use crate::{map_block_convert_macro, Float};

/// Turn positive Float values into binary `1u8`, and negative into `0u8`.
pub struct BinarySlicer {
    src: ReadStreamp<Float>,
    dst: Streamp<u8>,
}

impl BinarySlicer {
    /// Create new binary slicer.
    pub fn new(src: ReadStreamp<Float>) -> Self {
        Self {
            src,
            dst: new_streamp(),
        }
    }

    fn process_one(&self, a: Float) -> u8 {
        if a > 0.0 {
            1
        } else {
            0
        }
    }
}

map_block_convert_macro![BinarySlicer, u8];
