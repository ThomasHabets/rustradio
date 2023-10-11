//! Discard anything written to this block.
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Stream;
use crate::Error;

/// Discard anything written to this block.
pub struct NullSink<T>
where
    T: Copy,
{
    src: Arc<Mutex<Stream<T>>>,
}

impl<T: Default + Copy> NullSink<T> {
    /// Create new NullSink block.
    pub fn new(src: Arc<Mutex<Stream<T>>>) -> Self {
        Self { src }
    }
}

impl<T> Block for NullSink<T>
where
    T: Copy,
{
    fn block_name(&self) -> &'static str {
        "NullSink"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        self.src.lock().unwrap().clear();
        Ok(BlockRet::Ok)
    }
}
