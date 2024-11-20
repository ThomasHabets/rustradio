//! Turn positive Float values into binary `1u8`, and negative into `0u8`.
//!
//! TODO: should this be replaced with a MapBuilder, like in add_const?
use anyhow::Result;

use crate::stream::{Stream, Streamp};
use crate::Float;

/// Turn positive Float values into binary `1u8`, and negative into `0u8`.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out, sync)]
pub struct BinarySlicer {
    #[rustradio(in)]
    src: Streamp<Float>,
    #[rustradio(out)]
    dst: Streamp<u8>,
}

impl BinarySlicer {
    fn process_sync(&self, a: Float) -> u8 {
        if a > 0.0 {
            1
        } else {
            0
        }
    }
}
