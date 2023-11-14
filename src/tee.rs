//! Tee a stream

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
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
            dst1: new_streamp(),
            dst2: new_streamp(),
        }
    }
    /// Return the output streams.
    pub fn out(&self) -> (Streamp<T>, Streamp<T>) {
        (self.dst1.clone(), self.dst2.clone())
    }
}

impl<T: Copy> Block for Tee<T> {
    fn block_name(&self) -> &'static str {
        "Tee"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, tags) = self.src.read_buf()?;
        let mut o1 = self.dst1.write_buf()?;
        let mut o2 = self.dst2.write_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let n = std::cmp::min(i.len(), o1.len());
        let n = std::cmp::min(n, o2.len());
        o1.slice()[..n].clone_from_slice(&i.slice()[..n]);
        o2.slice()[..n].clone_from_slice(&i.slice()[..n]);
        o1.produce(n, &tags);
        o2.produce(n, &tags);
        i.consume(n);
        Ok(BlockRet::Ok)
    }
}
