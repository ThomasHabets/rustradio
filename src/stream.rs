/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::sync::Arc;

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
#[derive(Debug, PartialEq)]
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

/// A stream between blocks.
#[derive(Debug)]
pub struct Stream<T> {
    circ: circular_buffer::Buffer<T>,
}

pub trait ReadStream<T: Copy> {
    /// Return a read slice and the tags within the slice.
    ///
    /// The only reason for returning error should be if there's
    /// already a read slice handed out.
    fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>), Error>;
}

pub trait ReadStreamNoCopy<T> {
    /// Pop one sample.
    /// Ideally this should only be NoCopy.
    fn pop(&self) -> Option<T>;

    /// Get the size of the front packet.
    fn peek_size(&self) -> Option<usize>;
}
pub type ReadStreamp<T> = Arc<dyn ReadStream<T>>;
pub type ReadStreamNoCopyp<T> = Arc<dyn ReadStreamNoCopy<T>>;

/// Convenience type for a "pointer to a stream".
pub type Streamp<T> = Arc<Stream<T>>;

/// Create a new Streamp.
pub fn new_streamp<T>() -> Streamp<T> {
    Arc::new(Stream::new())
}

/// Create a new Streamp with contents.
pub fn streamp_from_slice<T: Copy>(data: &[T]) -> Streamp<T> {
    Arc::new(Stream::from_slice(data))
}

const DEFAULT_STREAM_SIZE: usize = 409600;

impl<T> Stream<T> {
    /// Create a new stream.
    pub fn new() -> Self {
        Self {
            circ: circular_buffer::Buffer::new(DEFAULT_STREAM_SIZE).unwrap(),
        }
    }

    /// Push one sample, handing off ownership.
    /// Ideally this should only be NoCopy.
    pub fn push(&self, val: T) {
        self.circ.push(val);
    }
}

impl<T: Copy> Stream<T> {
    /// Create a new stream with initial data in it.
    pub fn from_slice(data: &[T]) -> Self {
        let circ = circular_buffer::Buffer::new(DEFAULT_STREAM_SIZE).unwrap(); // TODO
        let mut wb = circ.write_buf().unwrap();
        wb.fill_from_slice(data);
        wb.produce(data.len(), &[]);
        Self { circ }
    }

    /// Return a write slice.
    ///
    /// The only reason for returning error should be if there's
    /// already a write slice handed out.
    pub fn write_buf(&self) -> Result<circular_buffer::BufferWriter<T>, Error> {
        // TODO: not sure why I need to use both Ok and ?. Should it not be From'd?
        Ok(self.circ.write_buf()?)
    }
}

impl<T: Copy> ReadStream<T> for Stream<T> {
    fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>), Error> {
        // TODO: not sure why I need to use both Ok and ?. Should it not be From'd?
        Ok(self.circ.read_buf()?)
    }
}
impl<T: Len> ReadStreamNoCopy<T> for Stream<T> {
    /// Pop one sample.
    /// Ideally this should only be NoCopy.
    fn pop(&self) -> Option<T> {
        self.circ.pop()
    }
    fn peek_size(&self) -> Option<usize> {
        self.circ.peek_size()
    }
}
impl<T> Default for Stream<T> {
    fn default() -> Self {
        Self::new()
    }
}
