//! Discard anything written to this block.

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, ReadStream};
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

/// Discard anything written to this block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct NCNullSink<T> {
    #[rustradio(in)]
    src: NCReadStream<T>,
}

impl<T: Send> Block for NCNullSink<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        while self.src.pop().is_some() {}
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}
