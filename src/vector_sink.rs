//! Sink values into a vector.
//!
//! This block is really only useful for unit tests.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag};
use crate::Error;

/// VectorSink.
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
        if n > 0 && self.tags.len() + tags.len() < self.max_size {
            self.storage.extend(&i.slice()[..n]);
            self.tags.extend(tags);
        }
        i.consume(ilen);
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
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
                Tag::new(0, "VectorSource::start".to_string(), TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat".to_string(), TagValue::U64(0)),
                Tag::new(0, "VectorSource::first".to_string(), TagValue::Bool(true)),
            ]
        );
        Ok(())
    }
}
