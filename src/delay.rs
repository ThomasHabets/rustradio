use anyhow::Result;

use crate::{Block, Sample, StreamReader, StreamWriter};

pub struct Delay {
    delay: usize,
}

impl Delay {
    pub fn new(delay: usize) -> Self {
        Self { delay }
    }
}

impl<T> Block<T> for Delay
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        r.set_history(self.delay);
        let n = std::cmp::min(r.available(), w.available());
        w.write(&r.buffer()[0..n])?;
        r.consume(n);
        Ok(())
    }
}
