//! Discard anything written to this block.
use anyhow::Result;

use crate::block::{get_input, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

pub struct NullSink<T> {
    _t: T,
}

impl<T: Default + Copy> NullSink<T> {
    pub fn new() -> Self {
        Self { _t: T::default() }
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
