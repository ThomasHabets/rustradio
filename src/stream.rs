/*! Streams connecting blocks.

Blocks are connected with streams. A block can have zero or more input
streams, and write to zero or more output streams.
*/
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use log::debug;

use crate::{Complex, Float};

/// A stream between blocks.
#[derive(Debug)]
pub struct Stream<T>
where
    T: Copy,
{
    data: VecDeque<T>,
    max_size: usize,
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
    pub fn new_from_slice(data: &[T]) -> Self {
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

impl From<StreamType> for Streamp<Float> {
    fn from(f: StreamType) -> Self {
        match f {
            StreamType::Float(x) => x,
            _ => panic!("tried to convert to Float {:?}", f),
        }
    }
}

impl From<StreamType> for Streamp<Complex> {
    fn from(f: StreamType) -> Self {
        match f {
            StreamType::Complex(x) => x,
            _ => panic!("tried to convert to Complex {:?}", f),
        }
    }
}

impl From<StreamType> for Streamp<u32> {
    fn from(f: StreamType) -> Self {
        match f {
            StreamType::U32(x) => x,
            _ => panic!("tried to convert to U32 {:?}", f),
        }
    }
}

impl From<StreamType> for Streamp<u8> {
    fn from(f: StreamType) -> Self {
        match f {
            StreamType::U8(x) => x,
            _ => panic!("tried to convert to U8 {:?}", f),
        }
    }
}

/// Shortcut type for a refcounted block.
pub type Streamp<T> = Rc<RefCell<Stream<T>>>;

/// StreamType is a stream of any supported type.
#[derive(Debug)]
pub enum StreamType {
    /// Stream is actually a disconnected port.
    Disconnected,

    /// Stream of floats.
    Float(Streamp<Float>),

    /// Stream of complex numbers, e.g. I/Q data.
    Complex(Streamp<Complex>),

    /// Stream of 32bit unsigned ints.
    U32(Streamp<u32>),

    /// Stream of 8bit unsigned ints, or bytes.
    U8(Streamp<u8>),
}

impl StreamType {
    /// Create new disconnected stream.
    pub fn new_disconnected() -> Self {
        Self::Disconnected
    }
    /// Create new stream of u8.
    pub fn new_u8() -> Self {
        Self::U8(Rc::new(RefCell::new(Stream::<u8>::new())))
    }
    /// Create new stream of u32.
    pub fn new_u32() -> Self {
        Self::U32(Rc::new(RefCell::new(Stream::<u32>::new())))
    }
    /// Create new stream of u32 with prepopulated data.
    pub fn new_u32_from_slice(data: &[u32]) -> Self {
        Self::U32(Rc::new(RefCell::new(Stream::<u32>::new_from_slice(data))))
    }
    /// Create new stream of floats.
    pub fn new_float() -> Self {
        Self::Float(Rc::new(RefCell::new(Stream::<Float>::new())))
    }
    /// Create new stream of floats with prepopulated data.
    pub fn new_float_from_slice(data: &[Float]) -> Self {
        Self::Float(Rc::new(RefCell::new(Stream::<Float>::new_from_slice(data))))
    }
    /// Create new stream of complex I/Q.
    pub fn new_complex() -> Self {
        Self::Complex(Rc::new(RefCell::new(Stream::<Complex>::new())))
    }
    /// Create new stream of complex I/Q with prepopulated data.
    pub fn new_complex_from_slice(data: &[Complex]) -> Self {
        Self::Complex(Rc::new(RefCell::new(Stream::<Complex>::new_from_slice(
            data,
        ))))
    }

    /// Return amount of data in stream.
    pub fn available(&self) -> usize {
        match &self {
            StreamType::Disconnected => 0,
            StreamType::Float(x) => x.borrow().available(),
            StreamType::U32(x) => x.borrow().available(),
            StreamType::U8(x) => x.borrow().available(),
            StreamType::Complex(x) => x.borrow().available(),
        }
    }

    /// Return amount of available space until full.
    pub fn capacity(&self) -> usize {
        match &self {
            StreamType::Disconnected => 0,
            StreamType::Float(x) => x.borrow().capacity(),
            StreamType::U32(x) => x.borrow().capacity(),
            StreamType::U8(x) => x.borrow().capacity(),
            StreamType::Complex(x) => x.borrow().capacity(),
        }
    }
}

impl Clone for StreamType {
    fn clone(&self) -> Self {
        match &self {
            Self::Disconnected => Self::Disconnected,
            Self::Float(x) => Self::Float(x.clone()),
            Self::Complex(x) => Self::Complex(x.clone()),
            Self::U32(x) => Self::U32(x.clone()),
            Self::U8(x) => Self::U8(x.clone()),
        }
    }
}

/// Wrapper for multiple input streams.
pub struct InputStreams {
    streams: Vec<StreamType>,
}

impl InputStreams {
    /// Create new InputStreams.
    pub fn new() -> Self {
        Self {
            streams: Vec::new(),
        }
    }

    /// Add a stream.
    pub fn add_stream(&mut self, s: StreamType) {
        self.streams.push(s);
    }

    /// Get a stream.
    pub fn get(&self, n: usize) -> StreamType {
        self.streams[n].clone()
    }

    /// Get number of streams (some may be disconnected).
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// True if no streams have been added.
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    /// Return the number of samples in one of the streams.
    pub fn available(&self, n: usize) -> usize {
        self.streams[n].available()
    }

    /// Sum up the number of samples in all streams.
    pub fn sum_available(&self) -> usize {
        self.streams.iter().map(|s| s.available()).sum()
    }

    /// Check if input `n` is of type Complex.
    pub fn is_complex(&self, n: usize) -> bool {
        matches!(self.streams[n], StreamType::Complex(_))
    }

    /// Check if input `n` is of type Float.
    pub fn is_float(&self, n: usize) -> bool {
        matches!(self.streams[n], StreamType::Float(_))
    }
}

impl Default for InputStreams {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper for multiple output streams.
pub struct OutputStreams {
    streams: Vec<StreamType>,
}

impl OutputStreams {
    /// Create new OutputStreams.
    pub fn new() -> Self {
        Self {
            streams: Vec::new(),
        }
    }

    /// Add stream.
    pub fn add_stream(&mut self, s: StreamType) {
        self.streams.push(s);
    }

    /// Get stream.
    pub fn get(&self, n: usize) -> StreamType {
        self.streams[n].clone()
    }

    /// Get number of streams (some may be disconnected).
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Return true if no streams have been added.
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    /// Return number of samples that can be added before stream is full.
    pub fn capacity(&self, n: usize) -> usize {
        match &self.streams[n] {
            StreamType::Disconnected => 0,
            StreamType::Float(x) => x.borrow().capacity(),
            StreamType::U32(x) => x.borrow().capacity(),
            StreamType::U8(x) => x.borrow().capacity(),
            StreamType::Complex(x) => x.borrow().capacity(),
        }
    }

    /// Sum up the number of samples in all streams.
    pub fn sum_available(&self) -> usize {
        self.streams.iter().map(|s| s.available()).sum()
    }

    /// Return true if all streams are at capacity.
    pub fn all_outputs_full(&self) -> bool {
        self.streams.iter().all(|s| s.capacity() == 0)
    }
}

impl Default for OutputStreams {
    fn default() -> Self {
        Self::new()
    }
}
