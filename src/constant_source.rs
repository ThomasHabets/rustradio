//! Generate the same value, forever.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

/// Generate the same value, forever.
pub struct ConstantSource<T> {
    val: T,
}

impl<T: Copy> ConstantSource<T> {
    /// Create a new ConstantSource block, providing the constant value.
    pub fn new(val: T) -> Self {
        Self { val }
    }
}

impl<T> Block for ConstantSource<T>
where
    T: Copy,
    Streamp<T>: From<StreamType>,
{
    fn block_name(&self) -> &'static str {
        "ConstantSource"
    }
    fn work(&mut self, _r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let n = w.capacity(0);
        w.get(0).borrow_mut().write_slice(&vec![self.val; n]);
        Ok(BlockRet::Ok)
    }
}
