//! Delay stream. Good for syncing up streams.
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, Sample};

/// Delay stream. Good for syncing up streams.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct Delay<T: Sample> {
    delay: usize,
    current_delay: usize,
    skip: usize,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample> Delay<T> {
    /// Create new Delay block.
    #[must_use]
    pub fn new(src: ReadStream<T>, delay: usize) -> (Self, ReadStream<T>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                delay,
                current_delay: delay,
                skip: 0,
            },
            dr,
        )
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

impl<T: Sample> Block for Delay<T> {
    fn work(&mut self) -> Result<BlockRet> {
        {
            let o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::Again);
            }
        }
        if self.current_delay > 0 {
            let mut o = self.dst.write_buf()?;
            let n = std::cmp::min(self.current_delay, o.len());
            if n == 0 {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
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
                return Ok(BlockRet::WaitForStream(&self.src, 1));
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
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TODO: test tag propagation.

    #[test]
    fn delay_zero() -> Result<()> {
        let s = ReadStream::from_slice(&[1.0f32, 2.0, 3.0]);
        let (mut delay, o) = Delay::new(s, 0);

        delay.work()?;
        let (res, _) = o.read_buf()?;
        assert_eq!(res.slice(), vec![1.0f32, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_one() -> Result<()> {
        let s = ReadStream::from_slice(&[1.0f32, 2.0, 3.0]);
        let (mut delay, o) = Delay::new(s, 1);

        delay.work()?;
        let (res, _) = o.read_buf()?;
        assert_eq!(res.slice(), vec![0.0f32, 1.0, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_change() -> Result<()> {
        // TODO: fix
        /*
        let s = ReadStream::from_slice(&[1u32, 2]);
        let (mut delay, o) = Delay::new(s, 1);

        delay.work()?;
        {
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
            let (res, _) = o.read_buf()?;
            assert_eq!(res.slice(), vec![0, 1, 2, 0, 3, 4, 7]);
        }
        */
        Ok(())
    }
}
