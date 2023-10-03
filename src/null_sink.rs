//! Discard anything written to this block.
use anyhow::Result;

use crate::block::{get_input, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

/// Discard anything written to this block.
pub struct NullSink<T> {
    dummy: std::marker::PhantomData<T>,
}

impl<T: Default + Copy> NullSink<T> {
    /// Create new NullSink block.
    pub fn new() -> Self {
        Self {
            dummy: std::marker::PhantomData,
        }
    }
}

impl<T: Default + Copy> Default for NullSink<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Block for NullSink<T>
where
    T: Copy,
    Streamp<T>: From<StreamType>,
{
    fn block_name(&self) -> &'static str {
        "NullSink"
    }
    fn work(&mut self, r: &mut InputStreams, _w: &mut OutputStreams) -> Result<BlockRet, Error> {
        get_input::<T>(r, 0).borrow_mut().clear();
        Ok(BlockRet::Ok)
    }
}
