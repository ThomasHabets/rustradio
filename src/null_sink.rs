//! Discard anything written to this block.

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;
use crate::Error;

/// Discard anything written to this block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct NullSink<T>
where
    T: Copy,
{
    #[rustradio(in)]
    src: ReadStream<T>,
}

impl<T: Default + Copy> NullSink<T> {
    /// Create new NullSink block.
    pub fn new(src: ReadStream<T>) -> Self {
        Self { src }
    }
}

impl<T> Block for NullSink<T>
where
    T: Copy,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _) = self.src.read_buf()?;
        let n = i.len();
        i.consume(n);
        // While we could discard in larger batches, making NullSink more
        // efficient, that risks needlessly blocking the previous block for lack
        // of output space.
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}
