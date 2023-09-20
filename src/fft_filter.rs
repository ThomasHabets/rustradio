use std::sync::Arc;

use anyhow::Result;
use rustfft::FftPlanner;

use crate::{Block, Complex, Float, StreamReader, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[test]
    fn blah() {
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

pub struct FftFilter {
    buf: Vec<Complex>,
    taps_fft: Vec<Complex>,
    len: usize,
    fft: Arc<dyn rustfft::Fft<Float>>,
    ifft: Arc<dyn rustfft::Fft<Float>>,
}

impl FftFilter {
    pub fn new(taps: &[Complex]) -> Self {
        let mut taps_fft = taps.to_vec();
        let len = taps_fft.len();
        //taps_fft.resize(2048, Complex::default());

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(len);
        let ifft = planner.plan_fft_inverse(len);

        //
        fft.process(&mut taps_fft);
        let f = 1.0 / taps_fft.len() as Float;
        taps_fft.iter_mut().for_each(|s: &mut Complex| *s *= f);

        let mut buf = Vec::new();
        buf.reserve(len);
        Self {
            len,
            buf,
            taps_fft,
            fft,
            ifft,
        }
    }
}

impl Block<Complex, Complex> for FftFilter {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Complex>,
        w: &mut dyn StreamWriter<Complex>,
    ) -> Result<()> {
        let add = std::cmp::min(r.available(), self.len - self.buf.len());
        for s in r.buffer() {
            if s.re > 1000.0 {
                panic!("wat?! {}", s);
            }
        }

        self.buf.extend(&r.buffer()[..add]);
        if self.buf.len() == self.len {
            self.fft.process(&mut self.buf);
            let mut filtered = self
                .buf
                .iter()
                .zip(self.taps_fft.iter())
                .map(|(x, y)| x * y)
                .collect::<Vec<Complex>>();
            self.ifft.process(&mut filtered);
            w.write(&filtered)?;
            self.buf.clear();
        }
        r.consume(add);
        Ok(())
    }
}
