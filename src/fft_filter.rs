//! FFT filter. Like a FIR filter, but more efficient when there are many taps.
use std::sync::Arc;

use anyhow::Result;
use rustfft::FftPlanner;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{Complex, Error, Float};

/// FFT filter. Like a FIR filter, but more efficient when there are many taps.
pub struct FftFilter {
    buf: Vec<Complex>,
    taps_fft: Vec<Complex>,
    nsamples: usize,
    fft_size: usize,
    tail: Vec<Complex>,
    fft: Arc<dyn rustfft::Fft<Float>>,
    ifft: Arc<dyn rustfft::Fft<Float>>,
}

impl FftFilter {
    fn calc_fft_size(from: usize) -> usize {
        let mut n = 1;
        while n < from {
            n <<= 1;
        }
        2 * n
    }

    /// Create new FftFilter, given filter taps.
    pub fn new(taps: &[Complex]) -> Self {
        // Set up FFT / batch size.
        let fft_size = Self::calc_fft_size(taps.len());
        let nsamples = fft_size - taps.len();

        // Create FFT planners.
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let ifft = planner.plan_fft_inverse(fft_size);

        // Pre-FFT the taps.
        let mut taps_fft = taps.to_vec();
        taps_fft.resize(fft_size, Complex::default());
        fft.process(&mut taps_fft);

        // Normalization is actually the square root of this
        // expression, but since we'll do two FFTs we can just skip
        // the square root here and do it just once here in setup.
        {
            let f = 1.0 / taps_fft.len() as Float;
            taps_fft.iter_mut().for_each(|s: &mut Complex| *s *= f);
        }

        Self {
            fft_size,
            taps_fft,
            tail: vec![Complex::default(); taps.len()],
            fft,
            ifft,
            buf: Vec::with_capacity(fft_size),
            nsamples,
        }
    }
}

impl Block for FftFilter {
    fn block_name(&self) -> &'static str {
        "FftFilter"
    }
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let input = r.get(0);
        loop {
            let add = std::cmp::min(input.borrow().available(), self.nsamples - self.buf.len());
            if add < self.nsamples {
                break;
            }
            self.buf.extend(input.borrow().iter().take(add));
            input.borrow_mut().consume(add);
            let o: Streamp<Complex> = w.get(0);
            if self.buf.len() == self.nsamples {
                self.buf.resize(self.fft_size, Complex::default());
                self.fft.process(&mut self.buf);
                let mut filtered: Vec<Complex> = self
                    .buf
                    .iter()
                    .zip(self.taps_fft.iter())
                    .map(|(x, y)| x * y)
                    .collect::<Vec<Complex>>();
                self.ifft.process(&mut filtered);

                // Add overlapping tail.
                for (i, t) in self.tail.iter().enumerate() {
                    filtered[i] += t;
                }

                // Output.
                o.borrow_mut().write_slice(&filtered[..self.nsamples]);

                // Stash tail.
                for i in 0..self.tail.len() {
                    self.tail[i] = filtered[self.nsamples + i];
                }

                self.buf.clear();
            }
        }
        Ok(BlockRet::Ok)
    }
}

/// FFT filter for float values.
pub struct FftFilterFloat {
    complex: FftFilter,
}

impl FftFilterFloat {
    /// Create a new FftFilterFloat block.
    pub fn new(taps: &[Float]) -> Self {
        let ctaps: Vec<Complex> = taps.iter().copied().map(|f| Complex::new(f, 0.0)).collect();
        Self {
            complex: FftFilter::new(&ctaps),
        }
    }
}

impl Block for FftFilterFloat {
    fn block_name(&self) -> &'static str {
        "FftFilterFloat"
    }
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let input: Vec<Complex> = r
            .get(0)
            .borrow()
            .iter()
            .copied()
            .map(|f| Complex::new(f, 0.0))
            .collect();
        let mut is = InputStreams::new();
        let mut os = OutputStreams::new();
        is.add_stream(StreamType::from_complex(&input));
        os.add_stream(StreamType::new_complex());
        let ret = self.complex.work(&mut is, &mut os)?;
        r.get_streamtype(0).consume(input.len() - is.available(0));
        let out: Vec<Float> = os
            .get(0)
            .borrow()
            .iter()
            .copied()
            .map(|c: Complex| c.re)
            .collect();
        w.get(0).borrow_mut().write_slice(&out);
        Ok(ret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[test]
    fn fft_then_ifft() {
        let buf = vec![
            Complex {
                re: -0.045228414,
                im: 0.05276649,
            },
            Complex {
                re: 0.045228407,
                im: 0.0075380574,
            },
            Complex {
                re: 0.015076124,
                im: 0.06030453,
            },
            Complex {
                re: -0.015076143,
                im: -0.03769034,
            },
            Complex {
                re: -0.045228403,
                im: 0.105532974,
            },
            Complex {
                re: -0.05276649,
                im: 0.17337555,
            },
            Complex {
                re: -0.06784265,
                im: 0.007538076,
            },
            Complex {
                re: -0.075380675,
                im: 0.08291874,
            },
            Complex {
                re: 5.3462292e-9,
                im: -0.03015228,
            },
            Complex {
                re: -0.015076158,
                im: 0.060304545,
            },
            Complex {
                re: -0.015076132,
                im: 0.10553298,
            },
            Complex {
                re: -0.045228425,
                im: 0.045228384,
            },
            Complex {
                re: 0.07538068,
                im: -0.0150761455,
            },
            Complex {
                re: 0.067842625,
                im: 0.01507613,
            },
            Complex {
                re: 0.0301523,
                im: 0.07538069,
            },
            Complex {
                re: -0.015076129,
                im: 0.067842595,
            },
            Complex {
                re: -0.015076145,
                im: -0.052766465,
            },
            Complex {
                re: -0.052766472,
                im: 0.022614188,
            },
            Complex {
                re: -0.09799488,
                im: 0.10553297,
            },
            Complex {
                re: -0.015076136,
                im: 0.0075380704,
            },
            Complex {
                re: -0.015076153,
                im: -4.566649e-9,
            },
            Complex {
                re: 0.06784262,
                im: -0.05276648,
            },
            Complex {
                re: 0.03015228,
                im: 0.022614205,
            },
            Complex {
                re: 0.09045683,
                im: -0.07538069,
            },
            Complex {
                re: 0.0075380807,
                im: -0.12814716,
            },
            Complex {
                re: 0.015076138,
                im: 0.06784261,
            },
            Complex {
                re: 0.075380705,
                im: -0.0150761455,
            },
            Complex {
                re: -0.060304567,
                im: -0.007538066,
            },
        ];
        let mut buf_fft = buf.clone();

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(buf.len());
        let ifft = planner.plan_fft_inverse(buf.len());
        fft.process(&mut buf_fft);
        ifft.process(&mut buf_fft);
        for i in 0..buf.len() {
            buf_fft[i] *= 1.0 / buf.len() as Float;
        }
        assert_eq!(buf.len(), buf_fft.len());
        assert_almost_equal_complex(&buf_fft, &buf);
    }
}
