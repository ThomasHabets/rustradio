use anyhow::Result;

use crate::{Sample, Sink, StreamReader};

pub struct DebugSink;

#[allow(clippy::new_without_default)]
impl DebugSink {
    pub fn new() -> Self {
        Self {}
    }
}

impl<T> Sink<T> for DebugSink
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
