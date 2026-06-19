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

    // Skip is the number of samples we're needlessly ahead. This can happen
    // when the delay changes mid stream.
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
        let lead = self
            .delay
            .saturating_add(self.skip)
            .saturating_sub(self.current_delay);
        self.current_delay = delay.saturating_sub(lead);
        self.skip = lead.saturating_sub(delay);
        self.delay = delay;
    }
}

impl<T: Sample> Block for Delay<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            // Check if we need to catch up.
            let (input, tags) = self.src.read_buf()?;
            if self.skip > 0 {
                let n = std::cmp::min(input.len(), self.skip);
                if n == 0 {
                    return Ok(BlockRet::WaitForStream(&self.src, 1));
                }
                input.consume(n);
                debug!("Delay: skipped {n}");
                self.skip -= n;
                continue;
            }

            // Everything except catch-up requires output space.
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }

            // Check if we're still delaying, thus filling with default.
            if self.current_delay > 0 {
                let n = std::cmp::min(self.current_delay, o.len());
                o.slice()[..n].fill(T::default());
                o.produce(n, &[]);
                self.current_delay -= n;
                continue;
            }

            // Neither skipping nor delaying. just plain copy.
            if input.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }

            let n = std::cmp::min(input.len(), o.len());
            assert_ne!(
                n, 0,
                "can't happen: we already checked both input and output"
            );
            o.fill_from_slice(&input.slice()[..n]);
            let tags = tags
                .into_iter()
                .filter(|tag| tag.pos() < n)
                .collect::<Vec<_>>();
            o.produce(n, &tags);
            input.consume(n);
        }
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
    fn delay_increase_before_work_extends_remaining_delay() -> Result<()> {
        let s = ReadStream::from_slice(&[1u32, 2]);
        let (mut delay, o) = Delay::new(s, 1);

        delay.set_delay(2);
        delay.work()?;
        let (res, _) = o.read_buf()?;
        assert_eq!(res.slice(), &[0, 0, 1, 2]);
        Ok(())
    }

    #[test]
    fn delay_decrease_before_work_reduces_remaining_delay() -> Result<()> {
        let s = ReadStream::from_slice(&[1u32, 2]);
        let (mut delay, o) = Delay::new(s, 2);

        delay.set_delay(1);
        delay.work()?;
        let (res, _) = o.read_buf()?;
        assert_eq!(res.slice(), &[0, 1, 2]);
        Ok(())
    }

    #[test]
    fn delay_change() -> Result<()> {
        let s = ReadStream::from_slice(&[1u32, 2]);
        let (mut delay, o) = Delay::new(s, 1);

        delay.work()?;
        {
            let (res, _) = o.read_buf()?;
            assert_eq!(res.slice(), vec![0, 1, 2]);
        }

        // TODO: fix
        /*
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
