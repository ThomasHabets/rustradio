//! Turn positive Float values into binary `1u8`, and negative into `0u8`.
use anyhow::Result;

use crate::stream::{new_streamp, Streamp};
use crate::{map_block_convert_macro, Float};

/// Turn positive Float values into binary `1u8`, and negative into `0u8`.
pub struct BinarySlicer {
    src: Streamp<Float>,
    dst: Streamp<u8>,
}

impl BinarySlicer {
    /// Create new binary slicer.
    pub fn new(src: Streamp<Float>) -> Self {
        Self {
            src,
            dst: new_streamp(),
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<u8> {
        self.dst.clone()
    }

    fn process_one(&self, a: Float) -> u8 {
        if a > 0.0 {
            1
        } else {
            0
        }
    }
}

map_block_convert_macro![BinarySlicer];
