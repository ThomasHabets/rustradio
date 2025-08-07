//! Discard anything written to this block.

use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;
use crate::{Result, Sample};

/// Discard anything written to this block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct NullSink<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
}

impl<T: Sample> Block for NullSink<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (i, _) = self.src.read_buf()?;
        let n = i.len();
        i.consume(n);
        // While we could discard in larger batches, making NullSink more
        // efficient, that risks needlessly blocking the previous block for lack
        // of output space.
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}
