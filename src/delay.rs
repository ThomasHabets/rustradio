use anyhow::Result;

use crate::{Block, Sample, StreamReader, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_sink::VectorSink;
    use crate::vector_source::VectorSource;
    use crate::{Source, Stream};

    #[test]
    fn delay_zero() -> Result<()> {
        let mut src = VectorSource::new(vec![1u32, 2, 3]);
        let mut sink: VectorSink<u32> = VectorSink::new();
        let mut s1 = Stream::new(10);
        let mut s2 = Stream::new(10);
        let mut delay = Delay::new(0);

        src.work(&mut s1)?;
        delay.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        assert_eq!(sink.to_vec(), vec![1u32, 2, 3]);
        Ok(())
    }

    #[test]
    fn delay_one() -> Result<()> {
        let mut src = VectorSource::new(vec![1u32, 2, 3]);
        let mut sink: VectorSink<u32> = VectorSink::new();
        let mut s1 = Stream::new(10);
        let mut s2 = Stream::new(10);
        let mut delay = Delay::new(1);

        src.work(&mut s1)?;
        delay.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        assert_eq!(sink.to_vec(), vec![0u32, 1, 2, 3]);
        Ok(())
    }

    #[test]
    fn delay_change() -> Result<()> {
        let mut src = VectorSource::new(vec![1u32, 2, 3, 4, 5, 6, 7]);
        let mut sink: VectorSink<u32> = VectorSink::new();
        let mut s1 = Stream::new(2);
        let mut s2 = Stream::new(10);
        let mut delay = Delay::new(1);

        // 1,2 => 0,1,2
        src.work(&mut s1)?;
        delay.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;

        // 3,4 => 0,3,4
        delay.set_delay(2);
        src.work(&mut s1)?;
        delay.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;

        // 5,6 => nothing
        delay.set_delay(0);
        src.work(&mut s1)?;
        delay.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;

        // 7 => 7
        src.work(&mut s1)?;
        delay.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;

        assert_eq!(sink.to_vec(), vec![0u32, 1, 2, 0, 3, 4, 7]);
        Ok(())
    }
}

pub struct Delay {
    delay: usize,
    current_delay: usize,
    skip: usize,
}

impl Delay {
    pub fn new(delay: usize) -> Self {
        Self {
            delay,
            current_delay: delay,
            skip: 0,
        }
    }
    pub fn set_delay(&mut self, delay: usize) {
        if delay > self.delay {
            self.current_delay = delay - self.delay;
        } else {
            let cdskip = std::cmp::min(self.current_delay, delay);
            self.current_delay -= cdskip;
            self.skip = (self.delay - delay) - cdskip;
            eprintln!("cdskip {cdskip} for {delay}");
        }
        self.delay = delay;
    }
}

impl<T> Block<T, T> for Delay
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        if self.current_delay > 0 {
            let n = std::cmp::min(self.current_delay, w.capacity());
            w.write(&vec![T::default(); n])?;
            self.current_delay -= n;
        }
        {
            let n = std::cmp::min(r.available(), self.skip);
            r.consume(n);
            eprintln!("========= skipped {n}");
            self.skip -= n;
        }

        let n = std::cmp::min(r.available(), w.capacity());
        w.write(&r.buffer()[0..n])?;
        r.consume(n);
        Ok(())
    }
}
