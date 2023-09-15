use anyhow::Result;

use crate::{Block, Complex, Float, StreamReader, StreamWriter};

pub struct ComplexToMag2;

impl ComplexToMag2 {
    pub fn new() -> Self {
        Self {}
    }
}

impl Block<Complex, Float> for ComplexToMag2 {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Complex>,
        w: &mut dyn StreamWriter<Float>,
    ) -> Result<()> {
        let n = std::cmp::min(r.available(), w.capacity());
        w.write(
            &r.buffer()
                .iter()
                .take(n)
                .map(|item| item.norm_sqr())
                .collect::<Vec<Float>>(),
        )?;
        r.consume(n);
        Ok(())
    }
}

impl Default for ComplexToMag2 {
    fn default() -> Self {
        Self::new()
    }
}
