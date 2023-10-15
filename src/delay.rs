//! Delay stream. Good for syncing up streams.
use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
use crate::Error;

/// Delay stream. Good for syncing up streams.
pub struct Delay<T: Copy> {
    delay: usize,
    current_delay: usize,
    skip: usize,
    src: Streamp<T>,
    dst: Streamp<T>,
}

impl<T: Copy> Delay<T> {
    /// Create new Delay block.
    pub fn new(src: Streamp<T>, delay: usize) -> Self {
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
        if self.current_delay > 0 {
            let n = std::cmp::min(self.current_delay, self.dst.lock()?.capacity());
            if n == 0 {
                return Ok(BlockRet::Noop);
            }
            self.dst.lock()?.write_slice(&vec![T::default(); n]);
            self.current_delay -= n;
        }
        {
            let a = self.src.lock()?.available();
            let n = std::cmp::min(a, self.skip);
            if n == 0 && a == 0 {
                return Ok(BlockRet::Noop);
            }
            self.src.lock()?.consume(n);
            debug!("========= skipped {n}");
            self.skip -= n;
        }
        self.dst.lock()?.write(self.src.lock()?.iter().copied());
        self.src.lock()?.clear();
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::streamp_from_slice;

    #[test]
    fn delay_zero() -> Result<()> {
        let s = streamp_from_slice(&[1.0f32, 2.0, 3.0]);
        let mut delay = Delay::new(s, 0);

        delay.work()?;
        let o = delay.out();
        let res = o.lock().unwrap();
        assert_eq!(*res.data(), vec![1.0f32, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_one() -> Result<()> {
        let s = streamp_from_slice(&[1.0f32, 2.0, 3.0]);
        let mut delay = Delay::new(s, 1);

        delay.work()?;
        let o = delay.out();
        let res = o.lock().unwrap();
        assert_eq!(*res.data(), vec![0.0f32, 1.0, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_change() -> Result<()> {
        let s = streamp_from_slice(&[1u32, 2]);
        let mut delay = Delay::new(s.clone(), 1);

        delay.work()?;
        {
            let o = delay.out();
            let res = o.lock().unwrap();
            assert_eq!(*res.data(), vec![0, 1, 2]);
        }

        // 3,4 => 0,3,4
        s.lock().unwrap().write([3, 4]);
        delay.set_delay(2);
        delay.work()?;
        {
            let o = delay.out();
            let res = o.lock().unwrap();
            assert_eq!(*res.data(), vec![0, 1, 2, 0, 3, 4]);
        }

        // 5,6 => 0,3,4
        s.lock().unwrap().write([5, 6]);
        delay.set_delay(0);
        delay.work()?;
        {
            let o = delay.out();
            let res = o.lock().unwrap();
            assert_eq!(*res.data(), vec![0, 1, 2, 0, 3, 4]);
        }

        // 7 => 7
        s.lock().unwrap().write([7]);
        delay.set_delay(0);
        delay.work()?;
        {
            let o = delay.out();
            let res = o.lock().unwrap();
            assert_eq!(*res.data(), vec![0, 1, 2, 0, 3, 4, 7]);
        }
        Ok(())
    }
}
