//! IIR filter.
use anyhow::Result;

use crate::{map_block_macro_v2, Float};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::stream::{InputStreams, OutputStreams, StreamType};
    use crate::{Complex, Error, Float};

    #[test]
    fn iir_ff() -> Result<()> {
        // TODO: create an actual test.
        let mut iir =
            SinglePoleIIRFilter::<Float>::new(0.2).ok_or(Error::new("alpha out of range"))?;
        let mut is = InputStreams::new();
        is.add_stream(StreamType::new_float_from_slice(&[0.1, 0.2]));
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_float());
        iir.work(&mut is, &mut os)?;
        Ok(())
    }

    #[test]
    fn iir_cc() -> Result<()> {
        // TODO: create an actual test.
        let mut iir =
            SinglePoleIIRFilter::<Complex>::new(0.2).ok_or(Error::new("alpha out of range"))?;
        let mut is = InputStreams::new();
        is.add_stream(StreamType::new_complex_from_slice(&[
            Complex::new(1.0, 0.1),
            Complex::default(),
        ]));
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_complex());
        iir.work(&mut is, &mut os)?;
        Ok(())
    }

    #[test]
    fn reject_bad_alpha() -> Result<()> {
        SinglePoleIIRFilter::<Float>::new(0.0).ok_or(Error::new("should accept 0.0"))?;
        SinglePoleIIRFilter::<Float>::new(0.1).ok_or(Error::new("should accept 0.1"))?;
        SinglePoleIIRFilter::<Float>::new(1.0).ok_or(Error::new("should accept 1.0"))?;
        if SinglePoleIIRFilter::<Float>::new(-0.1).is_some() {
            return Err(Error::new("should not accept -0.1").into());
        }
        if SinglePoleIIRFilter::<Float>::new(1.1).is_some() {
            return Err(Error::new("should not accept 1.1").into());
        }
        Ok(())
    }
}

struct SinglePoleIIR<Tout> {
    alpha: Float, // TODO: GNURadio uses double
    one_minus_alpha: Float,
    prev_output: Tout,
}

impl<Tout> SinglePoleIIR<Tout>
where
    Tout: Copy + Default + std::ops::Mul<Float, Output = Tout> + std::ops::Add<Output = Tout>,
{
    fn new(alpha: Float) -> Option<Self> {
        let mut r = Self {
            alpha: Float::default(),
            one_minus_alpha: Float::default(),
            prev_output: Tout::default(),
        };
        r.set_taps(alpha)?;
        Some(r)
    }
    fn filter<Tin>(&mut self, sample: Tin) -> Tout
    where
        Tin: Copy + std::ops::Mul<Float, Output = Tin> + std::ops::Add<Tout, Output = Tout>,
    {
        let o: Tout = sample * self.alpha + self.prev_output * self.one_minus_alpha;
        self.prev_output = o;
        o
    }
    fn set_taps(&mut self, alpha: Float) -> Option<()> {
        if !(0.0..=1.0).contains(&alpha) {
            return None;
        }
        self.alpha = alpha;
        self.one_minus_alpha = 1.0 - alpha;
        Some(())
    }
}

pub struct SinglePoleIIRFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    iir: SinglePoleIIR<T>,
}

impl<T> SinglePoleIIRFilter<T>
where
    T: Copy
        + Default
        + std::ops::Mul<Float, Output = T>
        + std::ops::Mul<T, Output = T>
        + std::ops::Add<T, Output = T>,
{
    pub fn new(alpha: Float) -> Option<Self> {
        Some(Self {
            iir: SinglePoleIIR::<T>::new(alpha)?,
        })
    }
    fn process_one(&mut self, a: &T) -> T {
        self.iir.filter(*a)
    }
}

map_block_macro_v2![
    SinglePoleIIRFilter<T>,
    std::ops::Add<Output = T>,
    Default,
    std::ops::Mul<T, Output = T>,
    std::ops::Mul<Float, Output = T>,
    std::ops::Add<T, Output = T>
];
