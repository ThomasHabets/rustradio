/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use async_trait::async_trait;

use crate::{Float, Len, Result};

/// Tag position in the current stream.
pub type TagPos = usize;

/// Enum of tag values.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TagValue {
    /// String value.
    String(String),

    /// Float value.
    Float(Float),

    /// Bool value.
    Bool(bool),

    /// U64 value.
    U64(u64),

    /// I64 value.
    I64(i64),
}

impl std::fmt::Display for TagValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            TagValue::String(s) => write!(f, "String:{s}"),
            TagValue::Float(s) => write!(f, "Float:{s}"),
            TagValue::Bool(s) => write!(f, "Bool:{s}"),
            TagValue::U64(s) => write!(f, "U64:{s}"),
            TagValue::I64(s) => write!(f, "I64:{s}"),
        }
    }
}

/// Tags associated with a stream.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone, PartialOrd)]
pub struct Tag {
    pos: TagPos,
    key: String,
    val: TagValue,
}

impl Tag {
    /// Create new tag.
    #[must_use]
    pub fn new<T: Into<String>>(pos: TagPos, key: T, val: TagValue) -> Self {
        Self {
            pos,
            key: key.into(),
            val,
        }
    }

    /// Get pos.
    ///
    /// Relative to the current window.
    #[must_use]
    pub fn pos(&self) -> TagPos {
        self.pos
    }

    /// Set pos.
    ///
    /// Relative to the current window.
    pub fn set_pos(&mut self, pos: TagPos) {
        self.pos = pos;
    }

    /// Get tag key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Get tag value.
    #[must_use]
    pub fn val(&self) -> &TagValue {
        &self.val
    }
}

/// Default stream size. Must be a multiple of the system page size.
///
/// Larger means better batching, but more RAM used. Twice as much virtual
/// address space as RAM is used.
///
/// Some experimentation with the multithreaded `GraphRunner` on 2025-02-15 with
/// ax25-1200-rx, in real time:
/// * 40_000KiB: 0.929s
/// *  4_000KiB: 1.066
/// *    400KiB: 1.228s
pub(crate) const DEFAULT_STREAM_SIZE: usize = 4_096_000;

const DEFAULT_NOCOPY_CAPACITY: usize = 1_000;

/// Wait on a stream.
///
/// For `ReadStream`, wait until there's enough to read.
/// For `WriteStream`, wait until there's enough to write something.
#[async_trait]
pub trait StreamWait {
    /// ID shared between read and write side.
    #[must_use]
    fn id(&self) -> usize;

    /// Wait for "a while" or until `need` samples are available/space available.
    ///
    /// Return true if `need` will *never* be satisfied, and blocks waiting for
    /// it should just go ahead and EOF.
    #[must_use]
    fn wait(&self, need: usize) -> bool;

    /// Return true if the other end of this stream is disconnected.
    #[must_use]
    fn closed(&self) -> bool;

    /// Wait for "a while" or until `need` samples are available/space available.
    ///
    /// Return true if `need` will *never* be satisfied, and blocks waiting for
    /// it should just go ahead and EOF.
    ///
    /// `must_use` implied by async/Future.
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> bool;
}

#[async_trait]
impl<T: Copy + Sync + Send + 'static> StreamWait for ReadStream<T> {
    fn id(&self) -> usize {
        self.circ.id()
    }
    fn wait(&self, need: usize) -> bool {
        self.wait_for_read(need)
    }
    fn closed(&self) -> bool {
        self.refcount() == 1
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> bool {
        let have = self.circ.wait_for_read_async(need).await;
        let r = Arc::strong_count(&self.circ);
        have < need && r == 1
    }
}

#[async_trait]
impl<T: Copy + Send + Sync> StreamWait for WriteStream<T> {
    fn id(&self) -> usize {
        self.circ.id()
    }
    fn wait(&self, need: usize) -> bool {
        self.wait_for_write(need)
    }
    fn closed(&self) -> bool {
        self.refcount() == 1
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> bool {
        self.circ.wait_for_write_async(need).await < need && Arc::strong_count(&self.circ) == 1
    }
}

/// `ReadStream` is the reading side of a stream.
///
/// From the `ReadStream` you can get windows into the current stream by calling
/// `read_buf()`.
#[derive(Debug)]
pub struct ReadStream<T> {
    circ: Arc<crate::sys::Buffer<T>>,
}

impl<T: Default + Copy> ReadStream<T> {
    /// Create a new stream with initial data in it.
    #[cfg(test)]
    #[must_use]
    pub fn from_slice(data: &[T]) -> Self {
        let circ = Arc::new(crate::sys::Buffer::new(DEFAULT_STREAM_SIZE).unwrap()); // TODO
        let mut wb = circ.clone().write_buf().unwrap();
        wb.fill_from_slice(data);
        wb.produce(data.len(), &[]);
        Self { circ }
    }

    /// Return total length of underlying circular buffer (before the
    /// mapping doubling).
    #[must_use]
    pub fn total_size(&self) -> usize {
        self.circ.total_size()
    }

    /// Return a `BufferReader` allowing you to read from the stream, and
    /// "consume" from it.
    ///
    /// Tags are returned in ascending position order, preserving the order of
    /// tags at the same position.
    ///
    /// Only one read window may be outstanding, until it is consumed or
    /// dropped. A second checkout returns an error.
    pub fn read_buf(&self) -> Result<(crate::sys::BufferReader<T>, Vec<Tag>)> {
        Arc::clone(&self.circ).read_buf()
    }
}
impl<T: Copy> ReadStream<T> {
    /// Return true if the needed number of samples will *never* arrive.
    #[must_use]
    pub fn wait_for_read(&self, need: usize) -> bool {
        self.circ.wait_for_read(need) < need && Arc::strong_count(&self.circ) == 1
    }

    /// Return true if the needed number of samples will *never* arrive.
    #[cfg(feature = "async")]
    #[must_use]
    pub async fn wait_for_read_async(&self, need: usize) -> bool {
        self.circ.wait_for_read_async(need).await < need && Arc::strong_count(&self.circ) == 1
    }
}

impl<T> ReadStream<T> {
    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        // Fast path.
        let refcount = Arc::strong_count(&self.circ);
        if refcount != 1 {
            return false;
        }
        // Refcount 1 means that that the WriteStream has closed. No more data is coming. So as
        // long as the buffer is empty, that's it then.
        self.circ.is_empty()
    }

    #[must_use]
    pub(crate) fn refcount(&self) -> usize {
        Arc::strong_count(&self.circ)
    }
}

/// The write part of a stream.
#[derive(Debug)]
pub struct WriteStream<T> {
    circ: Arc<crate::sys::Buffer<T>>,
}

impl<T: Default + Copy> WriteStream<T> {
    /// Create a stream pair, returning buffer creation errors.
    ///
    /// See [`new_stream`] for supported layouts and possible errors.
    pub fn new() -> Result<(WriteStream<T>, ReadStream<T>)> {
        new_stream()
    }
}

impl<T> StreamReadSide for WriteStream<T> {
    type ReadSide = ReadStream<T>;
}

impl<T: Copy> WriteStream<T> {
    /// Return free space in the stream, in samples.
    #[must_use]
    pub fn free(&self) -> usize {
        self.circ.free()
    }

    /// Return a `BufferWriter` for writing to the stream.
    ///
    /// Only one write window may be outstanding. A second checkout returns
    /// an error until the first window is produced or dropped. A read window
    /// may coexist with a write window because their sample ranges are disjoint.
    pub fn write_buf(&self) -> Result<crate::sys::BufferWriter<T>> {
        Arc::clone(&self.circ).write_buf()
    }

    #[must_use]
    pub fn wait_for_write(&self, need: usize) -> bool {
        self.circ.wait_for_write(need) < need && Arc::strong_count(&self.circ) == 1
    }

    #[cfg(feature = "async")]
    #[must_use]
    pub async fn wait_for_write_async(&self, need: usize) -> bool {
        self.circ.wait_for_write_async(need).await < need && Arc::strong_count(&self.circ) == 1
    }

    #[must_use]
    pub(crate) fn refcount(&self) -> usize {
        Arc::strong_count(&self.circ)
    }
}

/// Create a new stream for data elements that implements Copy.
///
/// That's not to say that a bunch of Copy happens, but that it makes sense to
/// create sync blocks that take samples by value.
///
/// Basically anything that GNU Radio would *not* call a message port.
/// Storage starts with valid default values; zero bits need not represent a
/// valid element.
///
/// # Errors
///
/// Returns buffer creation errors, including temporary-file or memory-mapping
/// failures and unsupported element layouts (such as zero-sized elements).
pub fn new_stream<T: Default + Copy>() -> Result<(WriteStream<T>, ReadStream<T>)> {
    let circ = Arc::new(crate::sys::Buffer::new(DEFAULT_STREAM_SIZE)?);
    Ok((WriteStream { circ: circ.clone() }, ReadStream { circ }))
}

struct NCEntry<T> {
    val: T,
    tags: Vec<Tag>,
}

struct NCInner<T> {
    lock: Mutex<VecDeque<NCEntry<T>>>,
    cv: Condvar,
    capacity: usize,

    // Waiting for read.
    #[cfg(feature = "async")]
    acvr: tokio::sync::Notify,

    // Waiting for write.
    #[cfg(feature = "async")]
    acvw: tokio::sync::Notify,
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCReadStream<T> {
    id: usize,
    inner: Arc<NCInner<T>>,
}

#[async_trait]
impl<T: Send> StreamWait for NCReadStream<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn wait(&self, need: usize) -> bool {
        let l = self
            .inner
            .cv
            .wait_timeout_while(
                self.inner.lock.lock().unwrap(),
                std::time::Duration::from_millis(100),
                |s| s.len() < need,
            )
            .unwrap();
        l.0.len() < need && Arc::strong_count(&self.inner) == 1
    }

    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> bool {
        if self.inner.lock.lock().unwrap().len() >= need {
            return false;
        }
        loop {
            // TODO: count down time, don't reset to same on every iteration.
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(100));
            tokio::select! {
                _ = sleep => break,
                _ = self.inner.acvr.notified() => {
                    if self.inner.lock.lock().unwrap().len() >= need {
                        return false;
                    }
                },
            }
        }
        self.inner.lock.lock().unwrap().len() < need && self.closed()
    }
    fn closed(&self) -> bool {
        Arc::strong_count(&self.inner) == 1
    }
}

#[async_trait]
impl<T: Send> StreamWait for NCWriteStream<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn wait(&self, need: usize) -> bool {
        let capacity = self.inner.capacity;
        let l = self
            .inner
            .cv
            .wait_timeout_while(
                self.inner.lock.lock().unwrap(),
                std::time::Duration::from_millis(100),
                |s| capacity.saturating_sub(s.len()) < need,
            )
            .unwrap();
        capacity.saturating_sub(l.0.len()) < need && self.closed()
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, need: usize) -> bool {
        let capacity = self.inner.capacity;
        if capacity.saturating_sub(self.inner.lock.lock().unwrap().len()) >= need {
            return false;
        }
        loop {
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(100));
            tokio::select! {
                _ = sleep => break,
                _ = self.inner.acvw.notified() => {
                    if capacity.saturating_sub(self.inner.lock.lock().unwrap().len()) >= need {
                        return false;
                    }
                },
            }
        }
        capacity.saturating_sub(self.inner.lock.lock().unwrap().len()) < need && self.closed()
    }
    fn closed(&self) -> bool {
        Arc::strong_count(&self.inner) == 1
    }
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCWriteStream<T> {
    id: usize,
    inner: Arc<NCInner<T>>,
}

/// Create a new stream for data elements that do not implement Copy.
///
/// This is likely going to be frames, packets, and (in GNU Radio) "messages",
/// which you would not want to just copy willy nilly.
#[must_use]
pub fn new_nocopy_stream<T>() -> (NCWriteStream<T>, NCReadStream<T>) {
    let inner = Arc::new(NCInner {
        lock: Mutex::new(VecDeque::new()),
        cv: Condvar::new(),
        capacity: DEFAULT_NOCOPY_CAPACITY,

        // Waiting for read.
        #[cfg(feature = "async")]
        acvr: tokio::sync::Notify::new(),

        // Waiting for write.
        #[cfg(feature = "async")]
        acvw: tokio::sync::Notify::new(),
    });
    let id = crate::NEXT_STREAM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    (
        NCWriteStream {
            id,
            inner: inner.clone(),
        },
        NCReadStream { id, inner },
    )
}

impl<T> NCReadStream<T> {
    /// Pop one owned item and its tags.
    ///
    /// Tags have positions relative to the item and are sorted in nondecreasing
    /// order. Equal-position tags preserve their supplied order.
    #[must_use]
    pub fn pop(&self) -> Option<(T, Vec<Tag>)> {
        let ret = self
            .inner
            .lock
            .lock()
            .unwrap()
            .pop_front()
            .map(|v| (v.val, v.tags));
        self.inner.cv.notify_all();
        #[cfg(feature = "async")]
        self.inner.acvw.notify_one();
        ret
    }

    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        if self.inner.lock.lock().unwrap().is_empty() {
            Arc::strong_count(&self.inner) == 1
        } else {
            false
        }
    }

    /// Return true is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let has = self.inner.lock.lock().unwrap().len();
        has == 0
    }
}

/// Trait that helps finding the read side type of a write stream.
///
/// Used to simplify macros. You're unlikely to want to use this directly.
pub trait StreamReadSide {
    type ReadSide;
}

impl<T> StreamReadSide for NCWriteStream<T> {
    type ReadSide = NCReadStream<T>;
}
impl<T> NCWriteStream<T> {
    /// Create a new stream pair.
    #[must_use]
    pub fn new() -> (NCWriteStream<T>, NCReadStream<T>) {
        new_nocopy_stream()
    }
    /// Push one sample, handing off ownership.
    /// Ideally this should only be `NoCopy`.
    ///
    /// This function doesn't enforce capacity. If there's a risk of
    /// overflowing, then check `remaining()` before pushing.
    ///
    /// Tags must be sorted by nondecreasing position. Tags at the same position
    /// retain their supplied order; this method stores tags without sorting.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if tag positions are not sorted.
    pub fn push<Tags: Into<Vec<Tag>>>(&self, val: T, tags: Tags) {
        let tags = tags.into();
        debug_assert!(
            tags.windows(2).all(|pair| pair[0].pos() <= pair[1].pos()),
            "no-copy stream tags must be sorted by position"
        );
        self.inner
            .lock
            .lock()
            .unwrap()
            .push_back(NCEntry { val, tags });
        self.inner.cv.notify_all();
        #[cfg(feature = "async")]
        self.inner.acvr.notify_waiters();
    }

    /// Remaining capacity.
    #[must_use]
    pub fn remaining(&self) -> usize {
        let has = self.inner.lock.lock().unwrap().len();
        self.inner.capacity.saturating_sub(has)
    }
}

impl<T: Len> NCReadStream<T> {
    /// Get the size of the front packet.
    #[must_use]
    pub fn peek_size(&self) -> Option<usize> {
        self.inner.lock.lock().unwrap().front().map(|e| e.val.len())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_are_exclusive_per_endpoint_and_released_on_drop() -> Result<()> {
        let (tx, rx) = new_stream::<u8>()?;
        let mut writer = tx.write_buf()?;
        assert!(tx.write_buf().is_err());
        let (reader, _) = rx.read_buf()?;
        assert!(rx.read_buf().is_err());
        // Opposite endpoints can hold windows at the same time.
        writer.fill_from_slice(&[1, 2]);
        writer.produce(2, &[]);
        assert!(reader.is_empty());
        drop(reader);
        let (reader, _) = rx.read_buf()?;
        assert_eq!(reader.slice(), [1, 2]);
        let writer = tx.write_buf()?;
        assert!(tx.write_buf().is_err());
        reader.consume(2);
        drop(writer);
        assert!(tx.write_buf().is_ok());
        assert!(rx.read_buf()?.0.is_empty());
        Ok(())
    }

    #[test]
    fn reader_cannot_consume_samples_produced_after_checkout() -> Result<()> {
        let (tx, rx) = new_stream::<u8>()?;
        let (reader, _) = rx.read_buf()?;
        let mut writer = tx.write_buf()?;
        writer.fill_from_slice(&[1]);
        writer.produce(1, &[]);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.consume(1))).is_err()
        );
        assert_eq!(rx.read_buf()?.0.slice(), [1]);
        Ok(())
    }

    #[test]
    fn writer_cannot_produce_space_freed_after_checkout() -> Result<()> {
        let (tx, rx) = new_stream::<u8>()?;
        let mut writer = tx.write_buf()?;
        let len = writer.len();
        writer.slice().fill(1);
        writer.produce(len, &[]);
        let writer = tx.write_buf()?;
        rx.read_buf()?.0.consume(1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.produce(1, &[])))
                .is_err()
        );
        assert_eq!(rx.read_buf()?.0.len(), len - 1);
        Ok(())
    }

    use super::*;

    #[test]
    fn nocopy_tags_preserve_nondecreasing_position_order() {
        let (writer, reader) = new_nocopy_stream();
        for tags in [
            vec![],
            vec![Tag::new(3, "single", TagValue::Bool(true))],
            vec![
                Tag::new(0, "first", TagValue::Bool(true)),
                Tag::new(0, "second", TagValue::Bool(false)),
                Tag::new(3, "last", TagValue::Bool(true)),
            ],
        ] {
            writer.push(42u8, tags.clone());
            assert_eq!(reader.pop(), Some((42, tags)));
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "no-copy stream tags must be sorted by position")]
    fn nocopy_push_rejects_unsorted_tags_in_debug_builds() {
        let (writer, _reader) = new_nocopy_stream();
        writer.push(
            42u8,
            [
                Tag::new(2, "later", TagValue::Bool(true)),
                Tag::new(1, "earlier", TagValue::Bool(true)),
            ],
        );
    }
}
