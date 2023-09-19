use anyhow::Result;

use crate::{Block, Complex, Float, StreamReader, StreamWriter};

pub struct RtlSdrDecode;

impl RtlSdrDecode {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for RtlSdrDecode {
    fn default() -> Self {
        Self::new()
    }
}

impl Block<u8, Complex> for RtlSdrDecode {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<u8>,
        w: &mut dyn StreamWriter<Complex>,
    ) -> Result<()> {
        let samples = r.available() - r.available() % 2;

        w.write(
            (0..samples)
                .step_by(2)
                .map(|e| {
                    Complex::new(
                        ((r.buffer()[e] as Float) - 127.0) * 0.008,
                        ((r.buffer()[e + 1] as Float) - 127.0) * 0.008,
                    )
                })
                .collect::<Vec<Complex>>()
                .as_slice(),
        )?;
        r.consume(samples);
        Ok(())
    }
}
