use anyhow::Result;

use crate::{Block, Complex, Float, StreamReader, StreamWriter};

pub struct QuadratureDemod {
    gain: Float,
}

impl QuadratureDemod {
    pub fn new(gain: Float) -> Self {
        Self { gain }
    }
}

impl Block<Complex, Float> for QuadratureDemod {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Complex>,
        w: &mut dyn StreamWriter<Float>,
    ) -> Result<()> {
        // TODO: fix this when there's history.
        let n = std::cmp::min(w.capacity(), r.available()) - 1;
        let input = r.buffer();
        let mut tmp = vec![Complex::default(); n];
        for i in 0..n {
            tmp[i] = input[i + 1] * input[i].conj();
        }
        let mut v = vec![0.0; n];
        for i in 0..n {
            v[i] = self.gain * tmp[i].im.atan2(tmp[i].re);
        }
        r.consume(n);
        w.write(&v)
    }
}
