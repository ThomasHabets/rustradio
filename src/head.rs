//! Only forward first set of samples, then EOF.
use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};

/// Only forward first set of samples, then EOF.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Head<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
    remaining: u64,
}

impl<T: Sample + std::fmt::Debug> Block for Head<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            if self.remaining == 0 {
                return Ok(BlockRet::EOF);
            }
            let (i, tags) = self.src.read_buf()?;
            if i.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }

            let len = u64::try_from(i.len().min(o.len()))?;
            let len = len.min(self.remaining);
            self.remaining -= len;
            let len = usize::try_from(len)?;

            o.slice()[..len].copy_from_slice(&i.slice()[..len]);
            o.produce(len, &tags);
            i.consume(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::block::{Block, BlockRet};
    use crate::blocks::VectorSource;

    #[test]
    fn head() -> Result<()> {
        let indata = vec![1u8, 2, 3, 4, 5, 6];
        for first in 0..9 {
            let (mut ib, src) = VectorSource::new(indata.clone());
            ib.work()?;
            let (mut b, out) = Head::new(src, first);
            let ret = b.work()?;
            if usize::try_from(first)? <= indata.len() {
                assert!(matches![ret, BlockRet::EOF], "{ret:?}");
            } else {
                assert!(matches![ret, BlockRet::WaitForStream(_, 1)], "{ret:?}");
            }
            drop(ret);
            let (res, _) = out.read_buf()?;
            let got = res.slice().to_vec();
            let want = [1u8, 2, 3, 4, 5, 6];
            let want = &want[..usize::try_from(first)?.min(indata.len())];
            assert_eq!(got, want, "Head value {first}");
        }
        Ok(())
    }
}
