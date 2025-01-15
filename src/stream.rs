/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::circular_buffer;
use crate::{Error, Float, Len};

/// Tag position in the current stream.
pub type TagPos = usize;

/// Enum of tag values.
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Debug, PartialEq, Clone)]
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

    /// Get tag key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Get tag value.
    pub fn val(&self) -> &TagValue {
        &self.val
    }
}

pub(crate) const DEFAULT_STREAM_SIZE: usize = 409600;

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
    pub fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>), Error> {
        Ok(Arc::clone(&self.circ).read_buf()?)
    }

    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        // Fast path.
        if Arc::strong_count(&self.circ) != 1 {
            return false;
        }
        // TODO: can we remove this needless clone?
        match Arc::clone(&self.circ).read_buf() {
            Ok((b, _)) if !b.is_empty() => false,
            Err(_) => false,
            Ok(_) => Arc::strong_count(&self.circ) == 1,
        }
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
    pub fn write_buf(&self) -> Result<circular_buffer::BufferWriter<T>, Error> {
        Ok(Arc::clone(&self.circ).write_buf()?)
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
    q: Arc<Mutex<VecDeque<T>>>,
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NCWriteStream<T> {
    q: Arc<Mutex<VecDeque<T>>>,
}

/// Create a new stream for data elements that do not implement Copy.
///
/// This is likely going to be frames, packets, and (in GNU Radio) "messages",
/// which you would not want to just copy willy nilly.
#[must_use]
pub fn new_nocopy_stream<T>() -> (NCWriteStream<T>, NCReadStream<T>) {
    let q = Arc::new(Mutex::new(VecDeque::new()));
    (NCWriteStream { q: q.clone() }, NCReadStream { q })
}

impl<T> NCReadStream<T> {
    /// Pop one sample.
    /// Ideally this should only be NoCopy.
    #[must_use]
    pub fn pop(&self) -> Option<(T, Vec<Tag>)> {
        // TODO: attach tags.
        self.q.lock().unwrap().pop_front().map(|v| (v, Vec::new()))
    }

    /// Return true if there is nothing more ever to read from the stream.
    #[must_use]
    pub fn eof(&self) -> bool {
        Arc::strong_count(&self.q) == 1
    }
}

impl<T> NCWriteStream<T> {
    /// Push one sample, handing off ownership.
    /// Ideally this should only be NoCopy.
    ///
    /// TODO: Actually store the tags.
    pub fn push(&self, val: T, _tags: &[Tag]) {
        self.q.lock().unwrap().push_back(val);
    }
}

impl<T: Len> NCReadStream<T> {
    /// Get the size of the front packet.
    pub fn peek_size(&self) -> Option<usize> {
        self.q.lock().unwrap().front().map(|e| e.len())
    }
}
