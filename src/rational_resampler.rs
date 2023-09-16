/*
* Unlike the rational resampler in GNURadio, this one doesn't filter.
*/
use anyhow::Result;

use crate::{Block, Complex, StreamReader, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_sink::VectorSink;
    use crate::vector_source::VectorSource;
    use crate::{Complex, Float, Stream};

    fn runtest(inputsize: usize, interp: usize, deci: usize, finalcount: usize) -> Result<()> {
        let input: Vec<_> = (0..inputsize)
            .map(|i| Complex::new(i as Float, 0.0))
            .collect();
        let mut src = VectorSource::new(input.clone());
        let mut resamp = RationalResampler::new(interp, deci)?;
        let mut sink = VectorSink::new();
        let mut s1 = Stream::new(8192);
        let mut s2 = Stream::new(8192);
        src.work(&mut s1)?;
        resamp.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        assert_eq!(finalcount, sink.to_vec().len(), "{:?}", sink.to_vec());
        Ok(())
    }

    #[test]
    fn foo() -> Result<()> {
        runtest(10, 1, 1, 10)?;
        runtest(10, 1, 2, 5)?;
        runtest(10, 2, 1, 20)?;
        runtest(100, 2, 3, 66)?;
        runtest(100, 3, 2, 150)?;
        runtest(100, 300, 200, 150)?;
        runtest(100, 200000, 1024000, 19)?;
        Ok(())
    }
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    a
}

pub struct RationalResampler {
    deci: i64,
    interp: i64,
    counter: i64,
}

impl RationalResampler {
    pub fn new(mut interp: usize, mut deci: usize) -> Result<Self> {
        let g = gcd(deci, interp);
        deci /= g;
        interp /= g;
        Ok(Self {
            interp: i64::try_from(interp)?,
            deci: i64::try_from(deci)?,
            counter: 0,
        })
    }
}

impl Block<Complex, Complex> for RationalResampler {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Complex>,
        w: &mut dyn StreamWriter<Complex>,
    ) -> Result<()> {
        // TODO: don't overblow the buffer.
        let n = std::cmp::min(w.capacity(), r.available());
        let input = r.buffer();
        let mut v = Vec::new();
        self.counter -= self.deci as i64;
        for s in &input[..n] {
            self.counter += self.interp as i64;
            while self.counter >= 0 {
                v.push(*s);
                self.counter -= self.deci as i64;
            }
        }
        r.consume(n);
        w.write(&v)
    }
}
