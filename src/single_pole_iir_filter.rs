use anyhow::Result;

use crate::{Float, StreamReader, StreamWriter};

struct SinglePoleIIR<Tout> {
    alpha: Float, // TODO: GNURadio uses double
    one_minus_alpha: Float,
    prev_output: Tout,
}

impl<Tout> SinglePoleIIR<Tout>
where
    Tout: Copy + Default + std::ops::Mul<Float, Output = Tout> + std::ops::Add<Output = Tout>,
    f32: std::ops::Mul<Tout, Output = Tout>,
{
    fn new(alpha: Float) -> Self {
        assert!(alpha > 0.0 && alpha < 1.0);
        let mut r = Self {
            alpha: Float::default(),
            one_minus_alpha: Float::default(),
            prev_output: Tout::default(),
        };
        r.set_taps(alpha);
        r
    }
    fn filter<Tin>(&mut self, sample: Tin) -> Tout
    where
        Tin: Copy + std::ops::Mul<Float, Output = Tin> + std::ops::Add<Tout, Output = Tout>,
    {
        let o: Tout = sample * self.alpha + self.one_minus_alpha * self.prev_output;
        self.prev_output = o;
        o
    }
    fn set_taps(&mut self, alpha: Float) {
        assert!(alpha > 0.0 && alpha < 1.0);
        self.alpha = alpha;
        self.one_minus_alpha = 1.0 - alpha;
    }
}

// TODO: only supports float output.
pub struct SinglePoleIIRFilter {
    iir: SinglePoleIIR<Float>, // TODO: GNURadio uses double, here
}

impl SinglePoleIIRFilter {
    pub fn new(alpha: Float) -> Self {
        Self {
            iir: SinglePoleIIR::new(alpha),
        }
    }
    pub fn work(
        &mut self,
        r: &mut dyn StreamReader<Float>,
        w: &mut dyn StreamWriter<Float>,
    ) -> Result<()> {
        let n = std::cmp::min(w.available(), r.available());
        w.write(
            &r.buffer()
                .iter()
                .take(n)
                .map(|item| self.iir.filter(*item))
                .collect::<Vec<Float>>(),
        )?;
        r.consume(n);
        Ok(())
    }
}
