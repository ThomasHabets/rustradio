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
    coeffs: Vec<Complex>,
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
        let mut taps = vec![Complex::default(); ntaps];
        taps[0] = Complex::new(1.0, 0.0);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                taps,
                coeffs: vec![Complex::default(); ntaps],
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
    fn work(&mut self) -> Result<BlockRet> {
        let (input, tags) = self.src.read_buf()?;
        let mut output = self.dst.write_buf()?;

        let is = input.slice();
        if is.len() < self.taps.len() {
            return Ok(BlockRet::WaitForStream(&self.src, self.taps.len()));
        }

        let os = output.slice();
        if os.len() < self.taps.len() {
            return Ok(BlockRet::WaitForStream(&self.dst, self.taps.len()));
        }

        let len = std::cmp::min(is.len(), os.len());
        let len = std::cmp::min(self.taps.len(), len);

        // Process samples using CMA.
        for i in 0..len {
            let sample = is[i];

            // Compute the error signal (e = |y|^2 - R)
            let error = {
                let magnitude = sample.norm_sqr();
                // TODO: should this clip to 1.0 for real and imag?
                magnitude - self.desired_modulus
            };

            // Update coefficients using the CMA rule.
            for (tap, coeff) in self.taps.iter_mut().zip(self.coeffs.iter()) {
                *tap += self.step_size * error * coeff.conj() * sample;
            }

            // Generate the output sample.
            let output_sample: Complex = self
                .taps
                .iter()
                .zip(input.iter())
                .map(|(t, &s)| t * s)
                .sum();
            os[i] = output_sample;
        }

        output.produce(len, &tags);
        input.consume(len);

        Ok(BlockRet::Again)
    }
}
