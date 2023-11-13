/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use log::debug;

use crate::circular_buffer;
use crate::{Error, Float};

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
    // Position of first element in `data`. If `tags` is empty then it
    // has no meaning, and can be set to an arbitrary value.
    pos: TagPos,

    data: VecDeque<T>,
    tags: VecDeque<Tag>,
    max_size: usize,
    circ: circular_buffer::Buffer<T>,
}

/// Convenience type for a "pointer to a stream".
pub type Streamp<T> = Arc<Mutex<Stream<T>>>;

/// Convenience type for a "pointer to a new type of stream".
pub type Streamp2<T> = Arc<Stream<T>>;

/// Create a new Streamp.
pub fn new_streamp<T>() -> Streamp<T> {
    Arc::new(Mutex::new(Stream::new()))
}

/// Create a new Streamp.
pub fn new_streamp2<T>() -> Streamp2<T> {
    Arc::new(Stream::new())
}

/// Create a new Streamp with contents.
pub fn streamp_from_slice<T: Copy>(data: &[T]) -> Streamp<T> {
    Arc::new(Mutex::new(Stream::from_slice(data)))
}

/// Create a new Streamp with contents.
pub fn streamp2_from_slice<T: Copy>(data: &[T]) -> Streamp2<T> {
    Arc::new(Stream::from_slice(data))
}

impl<T> Stream<T> {
    /// Create a new stream.
    pub fn new() -> Self {
        Self {
            pos: 0,
            data: VecDeque::new(),
            tags: VecDeque::new(),
            max_size: 1048576,
            circ: circular_buffer::Buffer::new(4096).unwrap(),
        }
    }

    /// Push one sample, handing off ownership.
    pub fn push(&mut self, val: T) {
        self.data.push_back(val);
    }

    /// Push one sample, handing off ownership.
    pub fn push2(&self, val: T) {
        self.circ.push(val);
    }

    /// Pop one sample.
    pub fn pop(&self) -> Option<T> {
        self.circ.pop()
    }

    /// Push one sample, with tags.
    pub fn push_tags(&mut self, val: T, tags: &[Tag]) {
        let ofs = self.pos + self.data.len() as TagPos;
        self.tags.extend(tags.iter().map(|t| Tag {
            pos: ofs,
            key: t.key.clone(),
            val: t.val.clone(),
        }));
        self.data.push_back(val);
    }

    /// Get iterator for reading.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }

    /// Get tags in window.
    pub fn tags(&self) -> Vec<Tag> {
        self.tags
            .iter()
            .map(|t| Tag {
                pos: t.pos - self.pos,
                key: t.key.clone(),
                val: t.val.clone(),
            })
            .collect()
    }
    /// Get raw data.
    pub fn data(&self) -> &VecDeque<T> {
        &self.data
    }

    /// Empty stream.
    pub fn clear(&mut self) {
        self.data.clear();
        self.tags.clear();
        self.pos = 0;
    }

    /// Remove samples from the beginning.
    pub fn consume(&mut self, n: usize) {
        self.data.drain(0..n);
        self.pos += n as TagPos;
        let mut d = 0;
        for t in &self.tags {
            if t.pos < n as TagPos {
                d += 1;
            }
        }
        self.tags.drain(0..d);
        if self.tags.is_empty() {
            self.pos = 0;
        }
    }

    /// Return the amount of data present.
    pub fn available(&self) -> usize {
        self.data.len()
    }

    /// Return true if stream is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Return the amount of room left until max size is reached.
    pub fn capacity(&self) -> usize {
        let avail = self.available();
        if self.max_size < avail {
            debug!("Over capacity {} > {}", avail, self.max_size);
            0
        } else {
            self.max_size - avail
        }
    }
}

impl<T: Copy> Stream<T> {
    /// Create a new stream with initial data in it.
    pub fn from_slice(data: &[T]) -> Self {
        Self {
            pos: 0,
            tags: VecDeque::new(),
            data: VecDeque::from(data.to_vec()),
            max_size: 1048576,
            circ: circular_buffer::Buffer::new(4096).unwrap(), // TODO
        }
    }

    /// Reterun a write slice.
    ///
    /// The only reason for returning error should be if there's
    /// already a write slice handed out.
    pub fn write_buf(&self) -> Result<circular_buffer::BufferWriter<T>, Error> {
        // TODO: not sure why I need to use both Ok and ?.
        Ok(self.circ.write_buf()?)
    }

    /// Return a read slice and the tags within the slice.
    ///
    /// The only reason for returning error should be if there's
    /// already a read slice handed out.
    pub fn read_buf(&self) -> Result<(circular_buffer::BufferReader<T>, Vec<Tag>), Error> {
        // TODO: not sure why I need to use both Ok and ?. Should it not be From'd?
        Ok(self.circ.read_buf()?)
    }

    // TODO: why can't a slice be turned into a suitable iterator?
    /// Write to stream from slice.
    pub fn write_slice(&mut self, data: &[T]) {
        self.data.extend(data);
    }

    /// Write to stream from iterator.
    pub fn write<I: IntoIterator<Item = T>>(&mut self, data: I) {
        self.data.extend(data);
    }

    /// Write to stream from iterator.
    pub fn write_tags<I: IntoIterator<Item = T>>(&mut self, data: I, tags: &[Tag]) {
        // TODO: debug_assert!(tags.is_sorted());
        let ofs = self.pos + self.data.len() as TagPos;
        self.data.extend(data);
        self.tags.extend(tags.iter().map(|t| Tag {
            pos: t.pos + ofs,
            key: t.key.clone(),
            val: t.val.clone(),
        }));
    }
}
impl<T> Default for Stream<T> {
    fn default() -> Self {
        Self::new()
    }
}
