//! Generate the same value, forever.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Streamp;
use crate::Error;

/// Generate the same value, forever.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, out, new)]
pub struct ConstantSource<T: Copy> {
    #[rustradio(out)]
    dst: Streamp<T>,
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
