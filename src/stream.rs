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

#[derive(Debug)]
pub struct ReadStream<T> {
    circ: Arc<circular_buffer::Buffer<T>>,
}

impl<T: Copy> ReadStream<T> {
    /// Create a new stream with initial data in it.
    #[cfg(test)]
    pub fn from_slice(data: &[T]) -> Self {
        let circ = Arc::new(circular_buffer::Buffer::new(DEFAULT_STREAM_SIZE).unwrap()); // TODO
        let mut wb = circ.write_buf().unwrap();
        wb.fill_from_slice(data);
        wb.produce(data.len(), &[]);
        Self { circ }
    }
    /// Return total length of underlying circular buffer (before the
    /// mapping doubling).
    pub fn total_size(&self) -> usize {
        self.circ.total_size()
    }

    pub fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>), Error> {
        Ok(Arc::clone(&self.circ).read_buf()?)
    }
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

#[derive(Debug)]
pub struct WriteStream<T> {
    circ: Arc<circular_buffer::Buffer<T>>,
}

impl<T: Copy> WriteStream<T> {
    pub fn write_buf(&self) -> Result<circular_buffer::BufferWriter<T>, Error> {
        Ok(Arc::clone(&self.circ).write_buf()?)
    }
}

pub fn new_stream<T>() -> (WriteStream<T>, ReadStream<T>) {
    let circ = Arc::new(circular_buffer::Buffer::new(DEFAULT_STREAM_SIZE).unwrap());
    (WriteStream { circ: circ.clone() }, ReadStream { circ })
}

/// A stream of noncopyable objects (e.g. Vec / PDUs).
pub struct NoCopyStream<T> {
    s: Mutex<VecDeque<T>>,
    eof: std::sync::atomic::AtomicBool,
}

/// Convenience type for a "pointer to a stream".
pub type NoCopyStreamp<T> = Arc<NoCopyStream<T>>;

pub(crate) const DEFAULT_STREAM_SIZE: usize = 409600;

impl<T> NoCopyStream<T> {
    /// Create new stream.
    pub fn new() -> Self {
        Self {
            s: Mutex::new(VecDeque::new()),
            eof: false.into(),
        }
    }

    /// Create new streamp.
    pub fn newp() -> NoCopyStreamp<T> {
        Arc::new(Self::new())
    }

    /// Push one sample, handing off ownership.
    /// Ideally this should only be NoCopy.
    ///
    /// TODO: Actually store the tags.
    pub fn push(&self, val: T, _tags: &[Tag]) {
        self.s.lock().unwrap().push_back(val);
    }

    /// Pop one sample.
    /// Ideally this should only be NoCopy.
    pub fn pop(&self) -> Option<(T, Vec<Tag>)> {
        // TODO: attach tags.
        self.s.lock().unwrap().pop_front().map(|v| (v, Vec::new()))
    }

    /// Set EOF status on stream.
    ///
    /// Stream won't really be EOF until all data is also read.
    pub fn set_eof(&self) {
        self.eof.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Return stream EOF status.
    pub fn eof(&self) -> bool {
        self.s.lock().unwrap().is_empty() && self.eof.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl<T> Default for NoCopyStream<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Len> NoCopyStream<T> {
    /// Get the size of the front packet.
    pub fn peek_size(&self) -> Option<usize> {
        self.s.lock().unwrap().front().map(|e| e.len())
    }
}
