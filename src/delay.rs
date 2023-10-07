//! Delay stream. Good for syncing up streams.
use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

/// Delay stream. Good for syncing up streams.
pub struct Delay<T> {
    delay: usize,
    current_delay: usize,
    skip: usize,
    dummy: std::marker::PhantomData<T>,
}

impl<T> Delay<T> {
    /// Create new Delay block.
    pub fn new(delay: usize) -> Self {
        Self {
            delay,
            current_delay: delay,
            skip: 0,
            dummy: std::marker::PhantomData,
        }
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
    Streamp<T>: From<StreamType>,
{
    fn block_name(&self) -> &'static str {
        "Delay"
    }
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        if self.current_delay > 0 {
            let n = std::cmp::min(self.current_delay, w.capacity(0));
            w.get(0).borrow_mut().write_slice(&vec![T::default(); n]);
            self.current_delay -= n;
        }
        {
            let n = std::cmp::min(r.available(0), self.skip);
            r.get(0).borrow_mut().consume(n);
            debug!("========= skipped {n}");
            self.skip -= n;
        }
        w.get(0)
            .borrow_mut()
            .write(r.get(0).borrow().iter().copied());
        r.get(0).borrow_mut().clear();
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Float;

    #[test]
    fn delay_zero() -> Result<()> {
        let mut delay = Delay::<Float>::new(0);
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_float(&[1.0f32, 2.0, 3.0]));
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_float());
        delay.work(&mut is, &mut os)?;
        let res: Streamp<Float> = os.get(0).into();
        assert_eq!(*res.borrow().data(), vec![1.0f32, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_one() -> Result<()> {
        let mut delay = Delay::<Float>::new(1);
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_float(&[1.0f32, 2.0, 3.0]));
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_float());
        delay.work(&mut is, &mut os)?;
        let res: Streamp<Float> = os.get(0).into();
        assert_eq!(*res.borrow().data(), vec![0.0f32, 1.0, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn delay_change() -> Result<()> {
        let mut delay = Delay::<u32>::new(1);
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_u32());

        // 1,2 => 0,1,2
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_u32(&[1u32, 2]));
        delay.work(&mut is, &mut os)?;

        // 3,4 => 0,3,4
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_u32(&[3u32, 4]));
        delay.set_delay(2);
        delay.work(&mut is, &mut os)?;

        // 5,6 => nothing
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_u32(&[5u32, 6]));
        delay.set_delay(0);
        delay.work(&mut is, &mut os)?;

        // 7 => 7
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_u32(&[7u32]));
        delay.work(&mut is, &mut os)?;

        let res: Streamp<u32> = os.get(0).into();
        assert_eq!(*res.borrow().data(), vec![0u32, 1, 2, 0, 3, 4, 7]);
        Ok(())
    }
}
