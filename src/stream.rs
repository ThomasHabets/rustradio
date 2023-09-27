use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::{Complex, Float};

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
    pub fn new() -> Self {
        Self {
            data: VecDeque::new(),
            max_size: 1048576,
        }
    }
    pub fn new_from_slice(data: &[T]) -> Self {
        Self {
            data: VecDeque::from(data.to_vec()),
            max_size: 1048576,
        }
    }

    // TODO: why can't a slice be turned into a suitable iterator?
    pub fn write_slice(&mut self, data: &[T]) {
        self.data.extend(data);
    }
    pub fn write<I: IntoIterator<Item = T>>(&mut self, data: I) {
        self.data.extend(data);
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }
    pub fn data(&self) -> &VecDeque<T> {
        &self.data
    }
    pub fn clear(&mut self) {
        self.data.clear();
    }
    pub fn consume(&mut self, n: usize) {
        self.data.drain(0..n);
    }
    pub fn available(&self) -> usize {
        self.data.len()
    }
    pub fn capacity(&self) -> usize {
        self.max_size - self.available()
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

pub type Streamp<T> = Rc<RefCell<Stream<T>>>;

#[derive(Debug)]
pub enum StreamType {
    Float(Streamp<Float>),
    Complex(Streamp<Complex>),
    U32(Streamp<u32>),
    U8(Streamp<u8>),
}

impl StreamType {
    pub fn new_u8() -> Self {
        Self::U8(Rc::new(RefCell::new(Stream::<u8>::new())))
    }
    pub fn new_float() -> Self {
        Self::Float(Rc::new(RefCell::new(Stream::<Float>::new())))
    }
    pub fn new_float_from_slice(data: &[Float]) -> Self {
        Self::Float(Rc::new(RefCell::new(Stream::<Float>::new_from_slice(data))))
    }
    pub fn new_complex() -> Self {
        Self::Complex(Rc::new(RefCell::new(Stream::<Complex>::new())))
    }
    pub fn new_complex_from_slice(data: &[Complex]) -> Self {
        Self::Complex(Rc::new(RefCell::new(Stream::<Complex>::new_from_slice(
            data,
        ))))
    }
}
impl Clone for StreamType {
    fn clone(&self) -> Self {
        match &self {
            Self::Float(x) => Self::Float(x.clone()),
            Self::Complex(x) => Self::Complex(x.clone()),
            Self::U32(x) => Self::U32(x.clone()),
            Self::U8(x) => Self::U8(x.clone()),
        }
    }
}

pub struct InputStreams {
    streams: Vec<StreamType>,
}
impl InputStreams {
    pub fn new() -> Self {
        Self {
            streams: Vec::new(),
        }
    }
    pub fn add_stream(&mut self, s: StreamType) {
        self.streams.push(s);
    }
    pub fn get(&self, n: usize) -> StreamType {
        self.streams[n].clone()
    }
}
impl Default for InputStreams {
    fn default() -> Self {
        Self::new()
    }
}

pub struct OutputStreams {
    streams: Vec<StreamType>,
}
impl OutputStreams {
    pub fn new() -> Self {
        Self {
            streams: Vec::new(),
        }
    }
    pub fn add_stream(&mut self, s: StreamType) {
        self.streams.push(s);
    }
    pub fn get(&self, n: usize) -> StreamType {
        self.streams[n].clone()
    }
}
impl Default for OutputStreams {
    fn default() -> Self {
        Self::new()
    }
}
