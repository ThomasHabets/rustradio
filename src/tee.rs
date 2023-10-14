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
        let mut i = self.src.lock()?;
        let mut o1 = self.dst1.lock()?;
        let mut o2 = self.dst1.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        o1.write(i.iter().copied());
        o2.write(i.iter().copied());
        i.clear();
        Ok(BlockRet::Ok)
    }
}
