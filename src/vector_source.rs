//! Generate values from a fixed vector.
use crate::Result;

use crate::Repeat;
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};

/// VectorSource builder.
pub struct VectorSourceBuilder<T: Copy> {
    block: VectorSource<T>,
    out: ReadStream<T>,
}

impl<T: Copy> VectorSourceBuilder<T> {
    /// New VectorSource builder.
    pub fn new(data: Vec<T>) -> Self {
        let (block, out) = VectorSource::new(data);
        Self { block, out }
    }
    /// Set a finite repeat count.
    pub fn repeat(mut self, r: Repeat) -> VectorSourceBuilder<T> {
        self.block.set_repeat(r);
        self
    }
    /// Build the VectorSource.
    pub fn build(self) -> (VectorSource<T>, ReadStream<T>) {
        (self.block, self.out)
    }
}

/// Generate values from a fixed vector.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct VectorSource<T>
where
    T: Copy,
{
    #[rustradio(out)]
    dst: WriteStream<T>,
    data: Vec<T>,
    repeat: Repeat,
    pos: usize,
}

impl<T: Copy> VectorSource<T> {
    /// Create new Vector Source block.
    ///
    /// Optionally the data can repeat.
    pub fn new(data: Vec<T>) -> (Self, ReadStream<T>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                dst,
                data,
                repeat: Repeat::finite(1),
                pos: 0,
            },
            dr,
        )
    }

    /// Set repeat status.
    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }
}

impl<T> Block for VectorSource<T>
where
    T: Copy,
{
    fn work(&mut self) -> Result<BlockRet> {
        if self.data.is_empty() {
            return Ok(BlockRet::EOF);
        }
        if self.repeat.done() {
            return Ok(BlockRet::EOF);
        }
        let mut tags = if self.pos == 0 {
            vec![
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(
                    0,
                    "VectorSource::repeat",
                    TagValue::U64(self.repeat.count()),
                ),
            ]
        } else {
            vec![]
        };
        if self.repeat.count() == 0 {
            tags.push(Tag::new(0, "VectorSource::first", TagValue::Bool(true)));
        }
        let mut os = self.dst.write_buf()?;
        if os.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let n = std::cmp::min(os.len(), self.data.len() - self.pos);
        os.fill_from_slice(&self.data[self.pos..(self.pos + n)]);
        os.produce(n, &tags);

        self.pos += n;
        if self.pos == self.data.len() {
            if !self.repeat.again() {
                return Ok(BlockRet::EOF);
            }
            self.pos = 0;
        }
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn empty() -> Result<()> {
        let (mut src, _) = VectorSource::<u8>::new(vec![]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        Ok(())
    }

    #[test]
    fn some() -> Result<()> {
        let (mut src, os) = VectorSource::new(vec![1u8, 2, 3]);
        let r = src.work()?;
        assert!(matches![r, BlockRet::EOF], "res: {r:?}");
        let (res, tags) = os.read_buf()?;
        assert_eq!(res.slice(), &[1, 2, 3]);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
            ]
        );
        Ok(())
    }

    #[test]
    fn repeat0() -> Result<()> {
        let (mut src, os) = VectorSourceBuilder::new(vec![1u8, 2, 3])
            .repeat(Repeat::finite(0))
            .build();
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (res, _) = os.read_buf()?;
        assert!(res.is_empty());
        Ok(())
    }

    #[test]
    fn repeat1() -> Result<()> {
        let (mut src, os) = VectorSourceBuilder::new(vec![1u8, 2, 3])
            .repeat(Repeat::finite(1))
            .build();
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (res, _) = os.read_buf()?;
        assert_eq!(res.slice(), &[1u8, 2, 3]);
        Ok(())
    }

    #[test]
    fn repeat2() -> Result<()> {
        let (mut src, os) = VectorSourceBuilder::new(vec![1u8, 2, 3])
            .repeat(Repeat::finite(2))
            .build();
        assert!(matches![src.work()?, BlockRet::Again]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (res, tags) = os.read_buf()?;
        assert_eq!(res.slice(), &[1u8, 2, 3, 1, 2, 3]);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                Tag::new(3, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(3, "VectorSource::repeat", TagValue::U64(1)),
            ]
        );
        Ok(())
    }

    #[test]
    fn repeat_infinite() -> Result<()> {
        let (mut src, os) = VectorSourceBuilder::new(vec![1u8, 2, 3])
            .repeat(Repeat::infinite())
            .build();
        for _ in 0..10 {
            assert!(matches![src.work()?, BlockRet::Again]);
        }
        let (res, tags) = os.read_buf()?;
        assert_eq!(
            res.slice(),
            (0..10).flat_map(|_| vec![1u8, 2, 3]).collect::<Vec<_>>()
        );
        assert_eq!(
            tags,
            (0usize..10)
                .flat_map(|n| {
                    let mut ret = vec![
                        Tag::new(n * 3, "VectorSource::start", TagValue::Bool(true)),
                        Tag::new(n * 3, "VectorSource::repeat", TagValue::U64(n as u64)),
                    ];
                    if n == 0 {
                        ret.push(Tag::new(n * 3, "VectorSource::first", TagValue::Bool(true)));
                    }
                    ret
                })
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn max() -> Result<()> {
        let (mut src, os) = VectorSource::new(vec![0u8; crate::stream::DEFAULT_STREAM_SIZE]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (res, _) = os.read_buf()?;
        assert_eq!(res.len(), crate::stream::DEFAULT_STREAM_SIZE);
        Ok(())
    }

    #[test]
    fn very_large() -> Result<()> {
        let (mut src, os) = VectorSource::new(vec![0u8; crate::stream::DEFAULT_STREAM_SIZE + 100]);
        assert!(matches![src.work()?, BlockRet::Again]);
        {
            let (res, _) = os.read_buf()?;
            assert_eq!(res.len(), crate::stream::DEFAULT_STREAM_SIZE);
        }
        assert!(matches![src.work()?, BlockRet::WaitForStream(_, _)]);
        {
            let (res, _) = os.read_buf()?;
            assert_eq!(res.len(), crate::stream::DEFAULT_STREAM_SIZE);
            res.consume(crate::stream::DEFAULT_STREAM_SIZE);
        }
        assert!(matches![src.work()?, BlockRet::EOF]);
        {
            let (res, _) = os.read_buf()?;
            assert_eq!(res.len(), 100);
        }
        Ok(())
    }
}
