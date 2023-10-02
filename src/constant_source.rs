//! Generate the same value, forever.
use anyhow::Result;

use crate::block::{get_output, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

pub struct ConstantSource<T> {
    val: T,
}

impl<T: Copy> ConstantSource<T> {
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
        get_output(w, 0)
            .borrow_mut()
            .write_slice(&vec![self.val; w.capacity(0)]);
        Ok(BlockRet::Ok)
    }
}
