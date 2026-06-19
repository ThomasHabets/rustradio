//! WIP. Completely untested.
//!
//! Links:
//! * <https://en.wikipedia.org/wiki/Blind_equalization>

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float, Result};

/// CMA Equalizer.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct CmaEqualizer {
    taps: Vec<Complex>,
    desired_modulus: Float,
    step_size: Float,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl CmaEqualizer {
    #[must_use]
    pub fn new(
        ntaps: usize,
        desired_modulus: Float,
        step_size: Float,
        src: ReadStream<Complex>,
    ) -> (Self, ReadStream<Complex>) {
        assert_ne!(ntaps, 0);
        let mut taps = vec![Complex::default(); ntaps];
        taps[0] = Complex::new(1.0, 0.0);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                taps,
                desired_modulus,
                step_size,
                src,
                dst,
            },
            dr,
        )
    }
}

impl Block for CmaEqualizer {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (input, tags) = self.src.read_buf()?;
        let mut output = self.dst.write_buf()?;

        let is = input.slice();
        if is.len() < self.taps.len() {
            return Ok(BlockRet::WaitForStream(&self.src, self.taps.len()));
        }

        let os = output.slice();
        if os.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }

        let len = std::cmp::min(is.len() - self.taps.len() + 1, os.len());

        // Process samples using CMA.
        for i in 0..len {
            let window = &is[i..i + self.taps.len()];

            // Generate the output sample.
            let output_sample: Complex = self.taps.iter().zip(window).map(|(&t, &s)| t * s).sum();
            os[i] = output_sample;

            // Compute the error signal (e = |y|^2 - R)
            let error = {
                let magnitude = output_sample.norm_sqr();
                // TODO: should this clip to 1.0 for real and imag?
                self.desired_modulus - magnitude
            };

            // Update coefficients using the CMA rule.
            for (tap, &sample) in self.taps.iter_mut().zip(window) {
                *tap += self.step_size * error * output_sample * sample.conj();
            }
        }

        let tags = tags
            .into_iter()
            .filter(|tag| tag.pos() < len)
            .collect::<Vec<_>>();
        output.produce(len, &tags);
        input.consume(len);

        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    #[test]
    fn output_window_slides() -> Result<()> {
        let input = [
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.0),
        ];
        let (mut b, out) = CmaEqualizer::new(2, 1.0, 0.0, ReadStream::from_slice(&input));

        assert!(matches![b.work()?, BlockRet::Again]);
        assert_eq!(out.read_buf()?.0.slice(), &input[..2]);
        Ok(())
    }
}
