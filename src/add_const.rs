use anyhow::Result;

use crate::{Block, Sample, StreamReader, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;
    use crate::vector_sink::VectorSink;
    use crate::vector_source::VectorSource;
    use crate::{Complex, Float, Source, Stream};

    #[test]
    fn floats() -> Result<()> {
        let mut src = VectorSource::new(vec![1.0 as Float, 2.1, -3.2, -10.0]);
        let mut add = AddConst::new(4.2 as Float);
        let mut sink = VectorSink::new();
        let mut s1 = Stream::new(100);
        let mut s2 = Stream::new(100);
        src.work(&mut s1)?;
        add.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        assert_almost_equal_float(&sink.to_vec(), &[5.2 as Float, 6.3, 1.0, -5.8]);
        Ok(())
    }

    #[test]
    fn complex() -> Result<()> {
        let mut src = VectorSource::new(vec![Complex::new(123.4, 321.9)]);
        let mut add = AddConst::new(Complex::new(-23.4, -0.91));
        let mut sink = VectorSink::new();
        let mut s1 = Stream::new(100);
        let mut s2 = Stream::new(100);
        src.work(&mut s1)?;
        add.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        assert_almost_equal_complex(&sink.to_vec(), &[Complex::new(100.0, 320.99)]);
        Ok(())
    }
}

pub struct AddConst<T> {
    val: T,
}

impl<T> AddConst<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + std::ops::Add<Output = T>,
{
    pub fn new(val: T) -> Self {
        Self { val }
    }
}

impl<T> Block<T, T> for AddConst<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + std::ops::Add<Output = T>,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let mut v: Vec<T> = Vec::new();
        for d in r.buffer().iter() {
            v.push(*d + self.val);
        }
        w.write(v.as_slice())?;
        r.consume(v.len());
        Ok(())
    }
}
