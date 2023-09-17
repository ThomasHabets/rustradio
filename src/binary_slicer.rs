use anyhow::Result;

use crate::{Block, Float, StreamReader, StreamWriter};

pub struct BinarySlicer;

impl BinarySlicer {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for BinarySlicer {
    fn default() -> Self {
        Self::new()
    }
}

impl Block<Float, u8> for BinarySlicer {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Float>,
        w: &mut dyn StreamWriter<u8>,
    ) -> Result<()> {
        w.write(
            r.buffer()
                .iter()
                .map(|f| if *f > 0.0 { 1u8 } else { 0u8 })
                .collect::<Vec<u8>>()
                .as_slice(),
        )?;
        r.consume(r.buffer().len());
        Ok(())
    }
}
