/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::circular_buffer;
use crate::{Error, Float, Len, Result};

/// Tag position in the current stream.
pub type TagPos = usize;

/// Enum of tag values.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
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
#[derive(Debug, PartialEq, Clone, PartialOrd)]
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
/// Some experimentation with the multithreaded GraphRunner on 2025-02-15 with
/// ax25-1200-rx, in real time:
/// * 40_000KiB: 0.929s
/// *  4_000KiB: 1.066
/// *    400KiB: 1.228s
pub(crate) const DEFAULT_STREAM_SIZE: usize = 4_096_000;

const DEFAULT_NOCOPY_CAPACITY: usize = 1_000;

/// Wait on a stream.
///
/// For ReadStream, wait until there's enough to read.
/// For WriteStream, wait until there's enough to write something.
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

    #[cfg(feature = "async")]
    #[must_use]
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

/// ReadStream is the reading side of a stream.
///
/// From the ReadStream you can get windows into the current stream by calling
/// `read_buf()`.
#[derive(Debug)]
pub struct ReadStream<T> {
    circ: Arc<circular_buffer::Buffer<T>>,
}

impl<T: Copy> ReadStream<T> {
    /// Create a new stream with initial data in it.
    #[cfg(test)]
    #[must_use]
    pub fn from_slice(data: &[T]) -> Self {
        let circ = Arc::new(circular_buffer::Buffer::new(DEFAULT_STREAM_SIZE).unwrap()); // TODO
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

    /// Return a BufferReader allowing you to read from the stream, and
    /// "consume" from it.
    ///
    /// See [`WriteStream::write_buf`] for details about the refcount checks.
    pub fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>)> {
        let refcount = Arc::strong_count(&self.circ);
        debug_assert!(refcount < 4, "read_buf() called with refcount {refcount}");
        if refcount > 3 {
            return Err(Error::msg(format!(
                "read_buf() called with refcount {refcount}"
            )));
        }
        Arc::clone(&self.circ).read_buf()
    }

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
    circ: Arc<circular_buffer::Buffer<T>>,
}

impl<T> WriteStream<T> {
    /// Create new stream pair.
    #[must_use]
    pub fn new() -> (WriteStream<T>, ReadStream<T>) {
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

    /// Return a BufferWriter for writing to the stream.
    ///
    /// Ideally having a BufferWriter active on a stream should be prevented
    /// statically, but I've not come up with a way to do that.
    ///
    /// Having `write_buf` hold on to a mutable reference won't work, because
    /// streams are owned by blocks, and blocks need to be able to call their
    /// own mutable methods.
    ///
    /// BufferWriters do get an Arc to the circ buffer, though, so there should
    /// never be more than four references:
    /// * The source block.
    /// * The destination block.
    /// * The source BufferWriter.
    /// * The destination BufferReader.
    ///
    /// So this function needs to be called when the refcount is 3 or lower.
    ///
    /// Having more than four references is a definite coding bug, and hopefully
    /// will be caught by MTGraph testing during development.
    ///
    /// The above also goes for [`ReadStream::read_buf`].
    pub fn write_buf(&self) -> Result<circular_buffer::BufferWriter<T>> {
        let refcount = Arc::strong_count(&self.circ);
        debug_assert!(refcount < 4, "write_buf() called with refcount {refcount}");
        if refcount > 3 {
            return Err(Error::msg(format!(
                "write_buf() called with refcount {refcount}"
            )));
        }
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
#[must_use]
pub fn new_stream<T>() -> (WriteStream<T>, ReadStream<T>) {
    let circ = Arc::new(circular_buffer::Buffer::new(DEFAULT_STREAM_SIZE).unwrap());
    (WriteStream { circ: circ.clone() }, ReadStream { circ })
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
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCReadStream<T> {
    id: usize,
    inner: Arc<NCInner<T>>,
}

#[async_trait]
impl<T: Send + Sync> StreamWait for NCReadStream<T> {
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
impl<T: Send + Sync> StreamWait for NCWriteStream<T> {
    fn id(&self) -> usize {
        self.id
    }
    fn wait(&self, _need: usize) -> bool {
        // TODO: actually wait.
        self.closed()
    }
    #[cfg(feature = "async")]
    async fn wait_async(&self, _need: usize) -> bool {
        // TODO: actually wait.
        self.closed()
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
    /// Pop one sample.
    /// Ideally this should only be NoCopy.
    #[must_use]
    pub fn pop(&self) -> Option<(T, Vec<Tag>)> {
        // TODO: attach tags.
        let ret = self
            .inner
            .lock
            .lock()
            .unwrap()
            .pop_front()
            .map(|v| (v.val, v.tags));
        self.inner.cv.notify_all();
        ret
    }

    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        if !self.inner.lock.lock().unwrap().is_empty() {
            false
        } else {
            Arc::strong_count(&self.inner) == 1
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
    /// Ideally this should only be NoCopy.
    ///
    /// This function doesn't enforce capacity. If there's a risk of
    /// overflowing, then check `remaining()` before pushing.
    pub fn push<Tags: Into<Vec<Tag>>>(&self, val: T, tags: Tags) {
        self.inner.lock.lock().unwrap().push_back(NCEntry {
            val,
            tags: tags.into(),
        });
        self.inner.cv.notify_all();
    }

    /// Remaining capacity.
    #[must_use]
    pub fn remaining(&self) -> usize {
        let has = self.inner.lock.lock().unwrap().len();
        self.inner.capacity - has
    }
}

impl<T: Len> NCReadStream<T> {
    /// Get the size of the front packet.
    #[must_use]
    pub fn peek_size(&self) -> Option<usize> {
        self.inner.lock.lock().unwrap().front().map(|e| e.val.len())
    }
}
