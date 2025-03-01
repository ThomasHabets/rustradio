/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use crate::circular_buffer;
use crate::{Error, Float, Len};

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
    pub fn new(pos: TagPos, key: String, val: TagValue) -> Self {
        Self { pos, key, val }
    }

    /// Get pos.
    ///
    /// Relative to the current window.
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
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Get tag value.
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

/// Wait on a stream.
///
/// For ReadStream, wait until there's enough to read.
/// For WriteStream, wait until there's enough to write something.
pub trait StreamWait {
    fn wait(&self, need: usize);
}
impl<T: Copy> StreamWait for ReadStream<T> {
    fn wait(&self, need: usize) {
        self.wait_for_read(need);
    }
}
impl<T: Copy> StreamWait for WriteStream<T> {
    fn wait(&self, need: usize) {
        self.wait_for_write(need);
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
    pub fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>), Error> {
        let refcount = Arc::strong_count(&self.circ);
        debug_assert!(refcount < 4, "read_buf() called with refcount {refcount}");
        if refcount > 3 {
            return Err(Error::new(&format!(
                "read_buf() called with refcount {refcount}"
            )));
        }
        Ok(Arc::clone(&self.circ).read_buf()?)
    }

    pub fn wait_for_read(&self, need: usize) {
        self.circ.wait_for_read(need);
    }

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
        let (b, _) = Arc::clone(&self.circ)
            .read_buf()
            .expect("can't happen: read_buf() failed");
        b.is_empty()
    }
}

/// The write part of a stream.
#[derive(Debug)]
pub struct WriteStream<T> {
    circ: Arc<circular_buffer::Buffer<T>>,
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
    pub fn write_buf(&self) -> Result<circular_buffer::BufferWriter<T>, Error> {
        let refcount = Arc::strong_count(&self.circ);
        debug_assert!(refcount < 4, "write_buf() called with refcount {refcount}");
        if refcount > 3 {
            return Err(Error::new(&format!(
                "write_buf() called with refcount {refcount}"
            )));
        }
        Ok(Arc::clone(&self.circ).write_buf()?)
    }

    pub fn wait_for_write(&self, need: usize) {
        self.circ.wait_for_write(need);
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

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCReadStream<T> {
    q: Arc<(Mutex<VecDeque<T>>, Condvar)>,
}

impl<T> StreamWait for NCReadStream<T> {
    fn wait(&self, need: usize) {
        let (lock, cv) = &*self.q;
        let _ = cv
            .wait_timeout_while(
                lock.lock().unwrap(),
                std::time::Duration::from_millis(100),
                |s| s.len() < need,
            )
            .unwrap();
    }
}

impl<T> StreamWait for NCWriteStream<T> {
    fn wait(&self, _need: usize) {
        // TODO: we should have a maximum, shouldn't we?
        // For now, as much room as you need.
    }
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCWriteStream<T> {
    q: Arc<(Mutex<VecDeque<T>>, Condvar)>,
}

/// Create a new stream for data elements that do not implement Copy.
///
/// This is likely going to be frames, packets, and (in GNU Radio) "messages",
/// which you would not want to just copy willy nilly.
#[must_use]
pub fn new_nocopy_stream<T>() -> (NCWriteStream<T>, NCReadStream<T>) {
    let q = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
    (NCWriteStream { q: q.clone() }, NCReadStream { q })
}

impl<T> NCReadStream<T> {
    /// Pop one sample.
    /// Ideally this should only be NoCopy.
    #[must_use]
    pub fn pop(&self) -> Option<(T, Vec<Tag>)> {
        let (lock, cv) = &*self.q;
        // TODO: attach tags.
        let ret = lock.lock().unwrap().pop_front().map(|v| (v, Vec::new()));
        cv.notify_all();
        ret
    }

    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        if !self.q.0.lock().unwrap().is_empty() {
            false
        } else {
            Arc::strong_count(&self.q) == 1
        }
    }
}

impl<T> NCWriteStream<T> {
    /// Push one sample, handing off ownership.
    /// Ideally this should only be NoCopy.
    ///
    /// TODO: Actually store the tags.
    pub fn push(&self, val: T, _tags: &[Tag]) {
        let (lock, cv) = &*self.q;
        // TODO: attach tags.
        lock.lock().unwrap().push_back(val);
        cv.notify_all();
    }
}

impl<T: Len> NCReadStream<T> {
    /// Get the size of the front packet.
    pub fn peek_size(&self) -> Option<usize> {
        self.q.0.lock().unwrap().front().map(|e| e.len())
    }
}
