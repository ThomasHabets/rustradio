use anyhow::Result;
use std::io::Read;

use crate::{Sample, StreamWriter};

pub struct FileSource {
    filename: String,
    f: std::fs::File,
    repeat: bool,
}

impl FileSource {
    pub fn new(filename: String, repeat: bool) -> Result<Self> {
        let f = std::fs::File::open(&filename)?;
        Ok(Self { filename, f,repeat })
    }
    pub fn work<T>(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()>
    where T: Copy + Sample<Type = T> + std::fmt::Debug
    {
        let mut buffer = Vec::new();
        self.f.read_to_end(&mut buffer)?;

        let n = buffer.len();
        let size = T::size();
        let mut v = Vec::new();
        for c in 0..(n/size) {
            let a = size * c;
            let b = a + size;
            v.push(T::parse(&buffer[a..b])?);
        }
        w.write(&v)
    }
}
