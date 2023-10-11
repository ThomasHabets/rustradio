//! Generate values from a fixed vector.
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Stream;
use crate::Error;

/// Generate values from a fixed vector.
pub struct VectorSource<T>
where
    T: Copy,
{
    dst: Arc<Mutex<Stream<T>>>,
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
            dst: Arc::new(Mutex::new(Stream::new())),
            data,
            repeat,
            pos: 0,
        }
    }
    pub fn out(&self) -> Arc<Mutex<Stream<T>>> {
        self.dst.clone()
    }
}

impl<T> Block for VectorSource<T>
where
    T: Copy + std::fmt::Debug,
{
    fn block_name(&self) -> &'static str {
        "VectorSource"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut out = self.dst.lock().unwrap();
        let n = std::cmp::min(out.capacity(), self.data.len() - self.pos);
        out.write_slice(&self.data[self.pos..(self.pos + n)]);
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
