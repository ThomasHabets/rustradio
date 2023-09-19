use anyhow::Result;

use crate::{Block, Float, StreamReader, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_sink::VectorSink;
    use crate::vector_source::VectorSource;
    use crate::{Complex, Error, Float, Source, Stream};

    #[test]
    fn iir_ff() -> Result<()> {
        // TODO: create an actual test.
        let mut src = VectorSource::new(vec![1f32, 2.0, 3.0]);
        let mut sink = VectorSink::new();
        let mut s1 = Stream::new(10);
        let mut s2 = Stream::new(10);
        let mut iir = SinglePoleIIRFilter::new(0.2).ok_or(Error::new("alpha out of range"))?;

        src.work(&mut s1)?;
        iir.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        //assert_eq!(sink.to_vec(), vec![1u32, 2, 3]);
        Ok(())
    }

    #[test]
    fn iir_cc() -> Result<()> {
        // TODO: create an actual test.
        let mut src = VectorSource::new(vec![Complex::default()]);
        let mut sink = VectorSink::new();
        let mut s1 = Stream::new(10);
        let mut s2 = Stream::new(10);
        let mut iir = SinglePoleIIRFilter::new(0.2).ok_or(Error::new("alpha out of range"))?;

        src.work(&mut s1)?;
        iir.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        //assert_eq!(sink.to_vec(), vec![1u32, 2, 3]);
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
    Float: std::ops::Mul<Tout, Output = Tout>,
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
        let o: Tout = sample * self.alpha + self.one_minus_alpha * self.prev_output;
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
    Float: std::ops::Mul<T, Output = T>,
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
    Float: std::ops::Mul<T, Output = T>,
{
    pub fn new(alpha: Float) -> Option<Self> {
        Some(Self {
            iir: SinglePoleIIR::<T>::new(alpha)?,
        })
    }
}

impl<T> Block<T, T> for SinglePoleIIRFilter<T>
where
    T: Copy
        + Default
        + std::ops::Mul<Float, Output = T>
        + std::ops::Mul<T, Output = T>
        + std::ops::Add<T, Output = T>,
    Float: std::ops::Mul<T, Output = T>,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let n = std::cmp::min(w.capacity(), r.available());
        w.write(
            &r.buffer()
                .iter()
                .take(n)
                .map(|item| self.iir.filter(*item))
                .collect::<Vec<T>>(),
        )?;
        r.consume(n);
        Ok(())
    }
}
