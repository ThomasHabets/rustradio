//! Generate the same value, forever.
use anyhow::Result;

use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::WriteStream;

/// Generate the same value, forever.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct ConstantSource<T: Copy> {
    #[rustradio(out)]
    dst: WriteStream<T>,
    val: T,
}

impl<T> Block for ConstantSource<T>
where
    T: Copy,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut o = self.dst.write_buf()?;
        o.slice().fill(self.val);
        let n = o.len();
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}
