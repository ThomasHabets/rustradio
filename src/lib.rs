use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub mod add_const;
pub mod block;
pub mod blocks;

pub type Float = f32;
pub type Complex = num::complex::Complex<Float>;

#[derive(Debug, Clone)]
pub struct Error {
    msg: String,
}

impl Error {
    pub fn new(msg: &str) -> Self {
        Self {
            msg: msg.to_string(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "RustRadio Error: {}", self.msg)
    }
}

impl std::error::Error for Error {}

pub struct Stream<T>
where
    T: Copy,
{
    data: VecDeque<T>,
}

impl<T> Stream<T>
where
    T: Copy,
{
    pub fn new() -> Self {
        Self {
            data: VecDeque::new(),
        }
    }
    pub fn new_from_slice(data: &[T]) -> Self {
        Self {
            data: VecDeque::from(data.to_vec()),
        }
    }
    pub fn write<I: IntoIterator<Item = T>>(&mut self, data: I) {
        self.data.extend(data);
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
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
            _ => panic!(),
        }
    }
}

impl From<StreamType> for Streamp<u32> {
    fn from(f: StreamType) -> Self {
        match f {
            StreamType::U32(x) => x,
            _ => panic!(),
        }
    }
}

pub type Streamp<T> = Rc<RefCell<Stream<T>>>;

pub enum StreamType {
    Float(Streamp<Float>),
    U32(Streamp<u32>),
}
impl Clone for StreamType {
    fn clone(&self) -> Self {
        match &self {
            Self::Float(x) => Self::Float(x.clone()),
            Self::U32(x) => Self::U32(x.clone()),
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
