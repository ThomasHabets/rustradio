//! Skip samples, then stream at full speed.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

/// Turn samples into text.
pub struct Skip<T: Copy> {
    src: Streamp<T>,
    dst: Streamp<T>,
    skip: usize,
}

impl<T: Copy> Skip<T> {
    /// Create new Skip block.
    pub fn new(src: Streamp<T>, skip: usize) -> Self {
        Self {
            src,
            dst: Stream::newp(),
            skip,
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T: Copy + std::fmt::Debug> Block for Skip<T> {
    fn block_name(&self) -> &str {
        "Skip"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;

        if self.skip == 0 {
            // Fast path, once skipping is done.
            let len = std::cmp::min(i.len(), o.len());
            o.slice()[..len].copy_from_slice(&i.slice()[..len]);
            o.produce(len, &tags);
            i.consume(len);
            return Ok(BlockRet::Ok);
        }

        let skip = std::cmp::min(self.skip, i.len());
        i.consume(skip);
        self.skip -= skip;
        Ok(BlockRet::Ok)
    }
}
