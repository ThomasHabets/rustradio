//! Sink values into a vector.
//!
//! This block is really only useful for unit tests.
use std::sync::{Arc, Mutex, MutexGuard};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag};
use crate::{Result, Sample};

/// VectorSink.
///
/// This block is really only useful for unit tests. It takes what comes from
/// the stream and just adds it to a vector. Tags are stored to another vector.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct VectorSink<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,

    #[rustradio(default)]
    storage: Arc<Mutex<(Vec<T>, Vec<Tag>)>>,

    /// Max number of samples and/or tags to store.
    max_size: usize,
}

/// Hook is a hook into getting the data and tags written to the VectorSink.
pub struct Hook<T: Sample> {
    inner: Arc<Mutex<(Vec<T>, Vec<Tag>)>>,
}
impl<T: Sample> Hook<T> {
    /// Get a locked read only reference to the samples and the data.
    #[must_use]
    pub fn data(&self) -> Data<'_, T> {
        Data {
            inner: self.inner.lock().unwrap(),
        }
    }
}

/// Lock a read only reference to the samples and tags written to the
/// VectorSink.
///
/// The VectorSink is unable to write anything new while the Data is alive.
pub struct Data<'a, T: Sample> {
    inner: MutexGuard<'a, (Vec<T>, Vec<Tag>)>,
}
impl<T: Sample> Data<'_, T> {
    /// Get a slice of the data written to the VectorSink.
    #[must_use]
    pub fn samples(&self) -> &[T] {
        &self.inner.0
    }
    /// Get a slice of the tags written to the VectorSink.
    #[must_use]
    pub fn tags(&self) -> &[Tag] {
        &self.inner.1
    }
}

impl<T: Sample> VectorSink<T> {
    /// Get a Hook into the data that will be written.
    #[must_use]
    pub fn hook(&self) -> Hook<T> {
        Hook {
            inner: self.storage.clone(),
        }
    }
}

impl<T: Sample> Block for VectorSink<T> {
    fn work(&mut self) -> Result<BlockRet> {
        let mut storage = self.storage.lock().unwrap();
        let (i, tags) = self.src.read_buf()?;
        let ilen = i.len();
        let n = std::cmp::min(ilen, self.max_size - storage.0.len());
        // Maybe limit number of tags, too?
        if n > 0 {
            storage.0.extend(&i.slice()[..n]);
            storage.1.extend(tags);
            i.consume(ilen);
        }
        Ok(BlockRet::WaitForStream(&self.src, 1))
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
        assert_eq!(sink.hook().data().samples(), &[0, 1, 2, 3, 4, 5]);
        assert_eq!(
            sink.hook().data().tags(),
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
        assert!(matches![r, BlockRet::WaitForStream(_, 1)], "Got {r:?}");
        drop(r);
        let r = sink.work()?;
        assert!(matches![r, BlockRet::WaitForStream(_, 1)], "Got {r:?}");
        drop(r);
        assert_eq!(sink.hook().data().samples(), &[0, 1, 2]);
        assert_eq!(
            sink.hook().data().tags(),
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
            ]
        );
        Ok(())
    }
}
