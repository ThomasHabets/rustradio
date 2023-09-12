use crate::{Sample, StreamReader};
use anyhow::Result;

pub struct DebugSink {}
impl DebugSink {
    pub fn new() -> Self {
        Self {}
    }
    pub fn work<T: Copy + Sample<Type = T> + std::fmt::Debug>(
        &mut self,
        r: &mut dyn StreamReader<T>,
    ) -> Result<()> {
        for d in r.buffer().clone().into_iter() {
            println!("debug: {:?}", d);
        }
        r.consume(r.buffer().len());
        Ok(())
    }
}
