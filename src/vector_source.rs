//! Generate values from a fixed vector.
use anyhow::Result;

use crate::block::{get_output, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

/// Generate values from a fixed vector.
pub struct VectorSource<T> {
    data: Vec<T>,
    repeat: bool,
    pos: usize,
}

impl<T: Copy + std::fmt::Debug> VectorSource<T> {
    /// Create new Vector Source block.
    ///
    /// Optionally the data can repeat.
    pub fn new(data: Vec<T>, repeat: bool) -> Self {
        Self {
            data,
            repeat,
            pos: 0,
        }
    }
}

impl<T> Block for VectorSource<T>
where
    T: Copy + std::fmt::Debug,
    Streamp<T>: From<StreamType>,
{
    fn block_name(&self) -> &'static str {
        "VectorSource"
    }
    fn work(&mut self, _r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let n = std::cmp::min(w.capacity(0), self.data.len() - self.pos);
        get_output(w, 0)
            .borrow_mut()
            .write_slice(&self.data[self.pos..(self.pos + n)]);
        self.pos += n;
        if self.pos == self.data.len() {
            if !self.repeat {
                return Ok(BlockRet::EOF);
            }
            self.pos = 0;
        }
        Ok(BlockRet::Ok)
    }
}
