use anyhow::Result;

use crate::{Sample, StreamWriter};

pub struct ConstantSource<T> {
    val: T,
}

impl<T: Copy + Sample<Type = T> + std::fmt::Debug> ConstantSource<T> {
    pub fn new(val: T) -> Self {
        Self { val }
    }
    pub fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        w.write(&vec![self.val; w.available()])
    }
}
