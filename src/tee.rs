//! Tee a stream

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

/// Tee
// TODO: make sync
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out)]
pub struct Tee<T: Copy> {
    #[rustradio(in)]
    src: Streamp<T>,
    #[rustradio(out)]
    dst1: Streamp<T>,
    #[rustradio(out)]
    dst2: Streamp<T>,
}

impl<T: Copy> Block for Tee<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, tags) = self.src.read_buf()?;
        let mut o1 = self.dst1.write_buf()?;
        let mut o2 = self.dst2.write_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let n = std::cmp::min(i.len(), o1.len());
        let n = std::cmp::min(n, o2.len());
        o1.fill_from_slice(&i.slice()[..n]);
        o2.fill_from_slice(&i.slice()[..n]);
        o1.produce(n, &tags);
        o2.produce(n, &tags);
        i.consume(n);
        Ok(BlockRet::Ok)
    }
}
