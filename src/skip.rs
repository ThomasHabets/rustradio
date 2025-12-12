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
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
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
                continue;
            }

            let skip = std::cmp::min(self.skip, i.len());
            i.consume(skip);
            self.skip -= skip;
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
    fn skip() -> Result<()> {
        for skip in 0..10 {
            let (mut ib, src) = VectorSource::new(vec![1u8, 2, 3, 4, 5, 6]);
            ib.work()?;
            let (mut b, out) = Skip::new(src, skip);
            let ret = b.work()?;
            assert!(matches![ret, BlockRet::WaitForStream(_, _)], "{ret:?}");
            drop(ret);
            let (res, _) = out.read_buf()?;
            let got = res.slice().to_vec();
            let want = [1u8, 2, 3, 4, 5, 6];
            let want = if skip > want.len() {
                &[]
            } else {
                &want[skip..]
            };
            assert_eq!(got, want, "Skip value {skip}");
        }
        Ok(())
    }
}
