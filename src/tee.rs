//! Tee a stream

use anyhow::Result;

use crate::block::{Block, BlockName, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

/// Tee
pub struct Tee<T: Copy> {
    src: Streamp<T>,
    dst1: Streamp<T>,
    dst2: Streamp<T>,
}

impl<T: Copy> Tee<T> {
    /// Create new Tee block.
    pub fn new(src: Streamp<T>) -> Self {
        Self {
            src,
            dst1: Stream::newp(),
            dst2: Stream::newp(),
        }
    }
    /// Return the output streams.
    pub fn out(&self) -> (Streamp<T>, Streamp<T>) {
        (self.dst1.clone(), self.dst2.clone())
    }
}

impl<T: Copy> BlockName for Tee<T> {
    fn block_name(&self) -> &str {
        "Tee"
    }
}
impl<T: Copy> Block for Tee<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, tags) = self.src.read_buf()?;
        let mut o1 = self.dst1.write_buf()?;
        let mut o2 = self.dst2.write_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let n = std::cmp::min(i.len(), o1.len());
        let n = std::cmp::min(n, o2.len());
        o1.fill_from_slice(&i.slice()[..n]);
        o2.fill_from_slice(&i.slice()[..n]);
        o1.produce(n, &tags);
        o2.produce(n, &tags);
        i.consume(n);
        Ok(BlockRet::Ok)
    }
}
