/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use log::debug;

/// A stream between blocks.
#[derive(Debug)]
pub struct Stream<T>
where
    T: Copy,
{
    data: VecDeque<T>,
    max_size: usize,
}

/// Convenience type for a "pointer to a stream".
pub type Streamp<T> = Arc<Mutex<Stream<T>>>;

/// Create a new Streamp.
pub fn new_streamp<T: Copy>() -> Streamp<T> {
    Arc::new(Mutex::new(Stream::new()))
}

/// Create a new Streamp with contents.
pub fn streamp_from_slice<T: Copy>(data: &[T]) -> Streamp<T> {
    Arc::new(Mutex::new(Stream::from_slice(data)))
}

impl<T> Stream<T>
where
    T: Copy,
{
    /// Create a new stream.
    pub fn new() -> Self {
        Self {
            data: VecDeque::new(),
            max_size: 1048576,
        }
    }

    /// Create a new stream with initial data in it.
    pub fn from_slice(data: &[T]) -> Self {
        Self {
            data: VecDeque::from(data.to_vec()),
            max_size: 1048576,
        }
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

    /// Get iterator for reading.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }

    /// Get raw data.
    pub fn data(&self) -> &VecDeque<T> {
        &self.data
    }

    /// Empty stream.
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Remove samples from the beginning.
    pub fn consume(&mut self, n: usize) {
        self.data.drain(0..n);
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
impl<T: Copy> Default for Stream<T> {
    fn default() -> Self {
        Self::new()
    }
}
