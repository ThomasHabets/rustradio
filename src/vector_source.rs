use anyhow::Result;

use crate::{Sample, Source, StreamWriter};

pub struct VectorSource<T> {
    data: Vec<T>,
}

impl<T: Copy + Sample<Type = T> + std::fmt::Debug> VectorSource<T> {
    pub fn new(data: Vec<T>) -> Self {
        Self { data }
    }
}

impl<T> Source<T> for VectorSource<T>
where
    T: Copy,
{
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let n = std::cmp::min(w.capacity(), self.data.len());
        w.write(&self.data[0..n])?;
        self.data.drain(0..n);
        Ok(())
    }
}
