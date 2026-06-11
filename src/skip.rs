//! Skip samples, then stream at full speed.
use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};

/// Skip `skip` samples, passing the rest through as is.
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

            // Skip if there's skipping to do.
            if self.skip > 0 {
                let n = self.skip.min(i.len());
                i.consume(n);
                self.skip -= n;
                continue;
            }

            // No more skipping to do. Just copy.
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }

            let len = std::cmp::min(i.len(), o.len());
            o.slice()[..len].copy_from_slice(&i.slice()[..len]);
            let tags = tags
                .into_iter()
                .filter(|tag| tag.pos() < len)
                .collect::<Vec<_>>();
            o.produce(len, &tags);
            i.consume(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockRet};
    use crate::blocks::VectorSource;
    use crate::stream::{Tag, TagValue};
    use crate::{Repeat, Result};

    #[test]
    fn skip() -> Result<()> {
        for skip in 0..10 {
            eprintln!("skip={skip}");
            let (mut ib, src) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6])
                .repeat(Repeat::finite(2))
                .build()?;
            while !matches![ib.work()?, BlockRet::EOF] {}
            let (mut b, out) = Skip::new(src, skip);
            let ret = b.work()?;
            assert!(matches![ret, BlockRet::WaitForStream(_, _)], "{ret:?}");
            let (res, tags) = out.read_buf()?;
            let got = res.slice().to_vec();
            let want = [1u8, 2, 3, 4, 5, 6, 1, 2, 3, 4, 5, 6];
            let want = if skip > want.len() {
                &[]
            } else {
                &want[skip..]
            };
            let want_tags = match skip {
                0 => vec![
                    Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                    Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                    Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                    Tag::new(6, "VectorSource::start", TagValue::Bool(true)),
                    Tag::new(6, "VectorSource::repeat", TagValue::U64(1)),
                ],
                1..=6 => vec![
                    Tag::new(6 - skip, "VectorSource::start", TagValue::Bool(true)),
                    Tag::new(6 - skip, "VectorSource::repeat", TagValue::U64(1)),
                ],
                _ => vec![],
            };
            assert_eq!(got, want, "Skip value {skip}");
            assert_eq!(tags, want_tags,);
        }
        Ok(())
    }
}
