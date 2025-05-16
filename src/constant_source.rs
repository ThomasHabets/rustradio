//! Generate the same value, forever.
use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::WriteStream;

/// Generate the same value, forever.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct ConstantSource<T: Sample> {
    #[rustradio(out)]
    dst: WriteStream<T>,
    val: T,
}

impl<T: Sample> Block for ConstantSource<T> {
    fn work(&mut self) -> Result<BlockRet> {
        let mut o = self.dst.write_buf()?;
        o.slice().fill(self.val);
        let n = o.len();
        o.produce(n, &[]);
        Ok(BlockRet::WaitForStream(&self.dst, 1))
    }
}
