use anyhow::Result;

use crate::{Complex, Float, StreamReader, StreamWriter};

pub struct ComplexToMag2;

impl ComplexToMag2 {
    pub fn new() -> Self {
        Self {}
    }
    pub fn work(
        &mut self,
        r: &mut dyn StreamReader<Complex>,
        w: &mut dyn StreamWriter<Float>,
    ) -> Result<()> {
        let n = std::cmp::min(r.available(), w.available());
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
