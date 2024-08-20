//! Generate the same value, forever.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

/// Generate the same value, forever.
pub struct ConstantSource<T: Copy> {
    dst: Streamp<T>,
    val: T,
}

impl<T: Copy> ConstantSource<T> {
    /// Create a new ConstantSource block, providing the constant value.
    pub fn new(val: T) -> Self {
        Self {
            val,
            dst: Stream::newp(),
        }
    }
    /// Return the output stream.
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T> Block for ConstantSource<T>
where
    T: Copy,
{
    fn block_name(&self) -> &str {
        "ConstantSource"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut o = self.dst.write_buf()?;
        o.slice().fill(self.val);
        let n = o.len();
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}
