use crate::{Float, StreamReader, StreamWriter};
use anyhow::Result;

pub struct FloatToU32 {
    scale: Float,
}
impl FloatToU32 {
    pub fn new(scale: Float) -> Self {
        Self { scale }
    }
    pub fn work(
        &mut self,
        r: &mut dyn StreamReader<Float>,
        w: &mut dyn StreamWriter<u32>,
    ) -> Result<()> {
        let v: Vec<u32> = r
            .buffer()
            .iter()
            .map(|e| (*e * self.scale) as u32)
            .collect();
        w.write(&v)
    }
}
