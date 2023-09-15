use anyhow::Result;

use crate::{Float, StreamReader, StreamWriter};

mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use crate::vector_sink::VectorSink;
    #[allow(unused_imports)]
    use crate::vector_source::VectorSource;
    #[allow(unused_imports)]
    use crate::{Complex, Float, Stream};

    #[test]
    fn iir_ff() -> Result<()> {
        // TODO: create an actual test.
        let mut src = VectorSource::new(vec![1f32, 2.0, 3.0]);
        let mut sink = VectorSink::new();
        let mut s1 = Stream::new(10);
        let mut s2 = Stream::new(10);
        let mut iir = SinglePoleIIRFilter::new(0.2);

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
        let mut iir = SinglePoleIIRFilter::new(0.2);

        src.work(&mut s1)?;
        iir.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        //assert_eq!(sink.to_vec(), vec![1u32, 2, 3]);
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
    pub fn new(alpha: Float) -> Self {
        Self {
            iir: SinglePoleIIR::<T>::new(alpha),
        }
    }
    pub fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let n = std::cmp::min(w.available(), r.available());
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
