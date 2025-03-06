//! Generate values from a fixed vector.
use anyhow::Result;

use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};

/// Repeat or counts.
pub enum Repeat {
    /// Repeat finite number of times. 0 Means no output at all. 1 is default.
    Finite(u64),

    /// Repeat forever.
    Infinite,
}

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
    pub fn repeat(mut self, r: u64) -> VectorSourceBuilder<T> {
        self.block.set_repeat(Repeat::Finite(r));
        self
    }
    /// Repeat the block forever.
    pub fn repeat_forever(mut self) -> VectorSourceBuilder<T> {
        self.block.set_repeat(Repeat::Infinite);
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
    repeat_count: u64,
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
                repeat: Repeat::Finite(1),
                pos: 0,
                repeat_count: 0,
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
    fn work(&mut self) -> Result<BlockRet, Error> {
        if self.data.is_empty() {
            return Ok(BlockRet::EOF);
        }
        if let Repeat::Finite(repeat) = self.repeat {
            if self.repeat_count == repeat {
                return Ok(BlockRet::EOF);
            }
        }
        let mut tags = if self.pos == 0 {
            vec![
                Tag::new(0, "VectorSource::start".to_string(), TagValue::Bool(true)),
                Tag::new(
                    0,
                    "VectorSource::repeat".to_string(),
                    TagValue::U64(self.repeat_count),
                ),
            ]
        } else {
            vec![]
        };
        if self.repeat_count == 0 {
            tags.push(Tag::new(
                0,
                "VectorSource::first".to_string(),
                TagValue::Bool(true),
            ));
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
            self.repeat_count += 1;
            self.pos = 0;
        }
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
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
        assert!(matches![src.work()?, BlockRet::Again]);
        let (res, _) = os.read_buf()?;
        assert_eq!(res.slice(), &[1, 2, 3]);
        Ok(())
    }

    #[test]
    fn max() -> Result<()> {
        let (mut src, os) = VectorSource::new(vec![0u8; crate::stream::DEFAULT_STREAM_SIZE]);
        assert!(matches![src.work()?, BlockRet::Again]);
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
        assert!(matches![src.work()?, BlockRet::Again]);
        {
            let (res, _) = os.read_buf()?;
            assert_eq!(res.len(), 100);
        }
        assert!(matches![src.work()?, BlockRet::EOF]);
        Ok(())
    }
}
