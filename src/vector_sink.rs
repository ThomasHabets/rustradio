//! Sink values into a vector.
//!
//! This block is really only useful for unit tests.
use anyhow::Result;

use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag};

/// VectorSink.
///
/// This block is really only useful for unit tests. It takes what comes from
/// the stream and just adds it to a vector. Tags are stored to another vector.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct VectorSink<T: Copy> {
    #[rustradio(in)]
    src: ReadStream<T>,

    #[rustradio(default)]
    storage: Vec<T>,

    #[rustradio(default)]
    tags: Vec<Tag>,

    /// Max number of samples and/or tags to store.
    max_size: usize,
}

impl<T: Copy> VectorSink<T> {
    pub fn data(&self) -> &[T] {
        &self.storage
    }
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }
}

impl<T: Copy> Block for VectorSink<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, tags) = self.src.read_buf()?;
        let ilen = i.len();
        let n = std::cmp::min(ilen, self.max_size - self.storage.len());
        // Maybe limit number of tags, too?
        if n > 0 {
            self.storage.extend(&i.slice()[..n]);
            self.tags.extend(tags);
            i.consume(ilen);
        }
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::stream::{Tag, TagValue};

    #[test]
    fn only_data() -> Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![0u32, 1, 2, 3, 4, 5]);
        let mut sink = VectorSink::new(src_out, 100);
        src.work()?;
        sink.work()?;
        assert_eq!(sink.data(), &[0, 1, 2, 3, 4, 5]);
        assert_eq!(
            sink.tags(),
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
            ]
        );
        Ok(())
    }

    #[test]
    fn maxed_out() -> Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![0u32, 1, 2, 3, 4, 5]);
        let mut sink = VectorSink::new(src_out, 3);
        let r = src.work()?;
        assert!(matches![r, BlockRet::EOF], "Got {r:?}");
        let r = sink.work()?;
        assert!(matches![r, BlockRet::Again], "Got {r:?}");
        drop(r);
        let r = sink.work()?;
        assert!(matches![r, BlockRet::Again], "Got {r:?}");
        drop(r);
        assert_eq!(sink.data(), &[0, 1, 2]);
        assert_eq!(
            sink.tags(),
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
            ]
        );
        Ok(())
    }
}
