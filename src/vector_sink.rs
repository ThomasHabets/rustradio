use anyhow::Result;

use crate::{Sample, Sink, StreamReader};

pub struct VectorSink<T> {
    data: Vec<T>,
}

impl<T: Copy + Sample<Type = T> + std::fmt::Debug> VectorSink<T> {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }
    pub fn to_vec(&self) -> &[T] {
        &self.data
    }
}

impl<T: Copy + Sample<Type = T> + std::fmt::Debug> Default for VectorSink<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Sink<T> for VectorSink<T>
where
    T: Copy,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>) -> Result<()> {
        self.data.extend(r.buffer());
        r.consume(r.buffer().len());
        Ok(())
    }
}
