//! Turn positive Float values into binary `1u8`, and negative into `0u8`.
//!
//! TODO: should this be replaced with a MapBuilder, like in add_const?
use anyhow::Result;

use crate::Float;
use crate::stream::{ReadStream, WriteStream};

/// Turn positive Float values into binary `1u8`, and negative into `0u8`.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct BinarySlicer {
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
}

impl BinarySlicer {
    fn process_sync(&self, a: Float) -> u8 {
        if a > 0.0 { 1 } else { 0 }
    }
}
