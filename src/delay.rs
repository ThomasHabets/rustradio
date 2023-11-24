//! Delay stream. Good for syncing up streams.
use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp, ReadStreamp};
use crate::Error;

/// Delay stream. Good for syncing up streams.
pub struct Delay<T: Copy> {
    delay: usize,
    current_delay: usize,
    skip: usize,
    src: ReadStreamp<T>,
    dst: Streamp<T>,
}

impl<T: Copy> Delay<T> {
    /// Create new Delay block.
    pub fn new(src: ReadStreamp<T>, delay: usize) -> Self {
        Self {
            src,
            dst: new_streamp(),
            delay,
            current_delay: delay,
            skip: 0,
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }

    /// Change the delay.
    pub fn set_delay(&mut self, delay: usize) {
        if delay > self.delay {
            self.current_delay = delay - self.delay;
        } else {
            let cdskip = std::cmp::min(self.current_delay, delay);
            self.current_delay -= cdskip;
            self.skip = (self.delay - delay) - cdskip;
        }
        self.delay = delay;
    }
}

impl<T> Block for Delay<T>
where
    T: Copy + Default,
{
    fn block_name(&self) -> &'static str {
        "Delay"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        {
            let o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::Ok);
            }
        }
        if self.current_delay > 0 {
            let mut o = self.dst.write_buf()?;
            let n = std::cmp::min(self.current_delay, o.len());
            if n == 0 {
                return Ok(BlockRet::Noop);
            }
            o.slice()[..n].fill(T::default());
            o.produce(n, &[]);
            self.current_delay -= n;
        }
        {
            let (input, _tags) = self.src.read_buf()?;
            let a = input.len();
            let n = std::cmp::min(a, self.skip);
            if n == 0 && a == 0 {
                return Ok(BlockRet::Noop);
            }
            input.consume(n);
            debug!("Delay: skipped {n}");
            self.skip -= n;
        }
        let mut o = self.dst.write_buf()?;
        let (input, tags) = self.src.read_buf()?;
        let n = std::cmp::min(input.len(), o.len());
        o.fill_from_slice(input.slice());
        o.produce(n, &tags);
        input.consume(n);
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::streamp_from_slice;

    // TODO: test tag propagation.

    #[test]
    fn delay_zero() -> Result<()> {
        let s = streamp_from_slice(&[1.0f32, 2.0, 3.0]);
        let mut delay = Delay::new(s, 0);

        delay.work()?;
        let o = delay.out();
        let (res, _) = o.read_buf()?;
        assert_eq!(res.slice(), vec![1.0f32, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_one() -> Result<()> {
        let s = streamp_from_slice(&[1.0f32, 2.0, 3.0]);
        let mut delay = Delay::new(s, 1);

        delay.work()?;
        let o = delay.out();
        let (res, _) = o.read_buf()?;
        assert_eq!(res.slice(), vec![0.0f32, 1.0, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_change() -> Result<()> {
        let s = streamp_from_slice(&[1u32, 2]);
        let mut delay = Delay::new(s.clone(), 1);

        delay.work()?;
        {
            let o = delay.out();
            let (res, _) = o.read_buf()?;
            assert_eq!(res.slice(), vec![0, 1, 2]);
        }

        // 3,4 => 0,3,4
        {
            let mut b = s.write_buf()?;
            b.fill_from_slice(&[3, 4]);
            b.produce(2, &[]);
        }
        delay.set_delay(2);
        delay.work()?;
        {
            let o = delay.out();
            let (res, _) = o.read_buf()?;
            assert_eq!(res.slice(), vec![0, 1, 2, 0, 3, 4]);
        }

        // 5,6 => 0,3,4
        {
            let mut b = s.write_buf()?;
            b.fill_from_slice(&[5, 6]);
            b.produce(2, &[]);
        }
        delay.set_delay(0);
        delay.work()?;
        {
            let o = delay.out();
            let (res, _) = o.read_buf()?;
            assert_eq!(res.slice(), vec![0, 1, 2, 0, 3, 4]);
        }

        // 7 => 7
        {
            let mut b = s.write_buf()?;
            b.slice()[0] = 7;
            b.produce(1, &[]);
        }
        delay.set_delay(0);
        delay.work()?;
        {
            let o = delay.out();
            let (res, _) = o.read_buf()?;
            assert_eq!(res.slice(), vec![0, 1, 2, 0, 3, 4, 7]);
        }
        Ok(())
    }
}
