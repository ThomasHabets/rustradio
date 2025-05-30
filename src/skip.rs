//! Skip samples, then stream at full speed.
use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};

/// Turn samples into text.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Skip<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
    skip: usize,
}

impl<T: Sample + std::fmt::Debug> Block for Skip<T> {
    fn work(&mut self) -> Result<BlockRet> {
        let (i, tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }

        if self.skip == 0 {
            // Fast path, once skipping is done.
            let len = std::cmp::min(i.len(), o.len());
            o.slice()[..len].copy_from_slice(&i.slice()[..len]);
            o.produce(len, &tags);
            i.consume(len);
            return Ok(BlockRet::Again);
        }

        let skip = std::cmp::min(self.skip, i.len());
        i.consume(skip);
        self.skip -= skip;
        Ok(BlockRet::Again)
    }
}
