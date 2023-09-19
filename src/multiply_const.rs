use crate::{Block, Sample, StreamReader, StreamWriter};
use anyhow::Result;

pub struct MultiplyConst<T> {
    val: T,
}

impl<T> MultiplyConst<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + std::ops::Mul<Output = T>,
{
    pub fn new(val: T) -> Self {
        Self { val }
    }
}

impl<T> Block<T, T> for MultiplyConst<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + std::ops::Mul<Output = T>,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let mut v: Vec<T> = Vec::new();
        for d in r.buffer().clone().iter() {
            v.push(*d * self.val);
        }
        w.write(v.as_slice())?;
        r.consume(v.len());
        Ok(())
    }
}
