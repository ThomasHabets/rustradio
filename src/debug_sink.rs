use anyhow::Result;

use crate::{Sample, Sink, StreamReader};

pub struct DebugSink<T> {
    _t: T, // TODO: remote this dummy.
}

#[allow(clippy::new_without_default)]
impl<T> DebugSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    pub fn new() -> Self {
        Self { _t: T::default() }
    }
}

impl<T> Sink<T> for DebugSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>) -> Result<()> {
        for d in r.buffer().clone().iter() {
            println!("debug: {:?}", d);
        }
        r.consume(r.buffer().len());
        Ok(())
    }
}
