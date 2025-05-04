use std::sync::Arc;

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream};
use crate::{Complex, Float, Result};

/// Run FFT on message.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Fft {
    #[rustradio(in)]
    src: NCReadStream<Vec<Complex>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<Complex>>,
    fft: Arc<dyn rustfft::Fft<Float>>,
}

impl Fft {
    fn process_one(&mut self, input: &[Complex]) -> Vec<Complex> {
        let mut out = input.to_vec();
        self.fft.process(&mut out);
        out
    }
}

impl Block for Fft {
    fn work(&mut self) -> Result<BlockRet> {
        loop {
            let Some((msg, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let out = self.process_one(&msg);
            self.dst.push(out, &tags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_nocopy_stream;

    #[test]
    fn zeroes() -> Result<()> {
        let mut planner = rustfft::FftPlanner::new();
        let size = 1024;
        let fft = planner.plan_fft_forward(size);
        let (root, r) = new_nocopy_stream();
        let (mut f, out) = Fft::new(r, fft);
        assert!(out.pop().is_none());
        assert!(matches![f.work()?, BlockRet::WaitForStream(_, 1)]);
        assert!(out.pop().is_none());
        root.push(vec![Complex::default(); size], &[]);
        assert!(matches![f.work()?, BlockRet::WaitForStream(_, 1)]);
        // Get the results.
        let (omsg, tags) = out.pop().unwrap();
        assert_eq!(omsg.len(), size);
        assert_eq!(omsg, vec![Complex::default(); size]);
        assert_eq!(tags, &[]);

        // Should be empty now.
        assert!(out.pop().is_none());
        Ok(())
    }
}
