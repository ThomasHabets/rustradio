/*! FFT filter. Like a FIR filter, but more efficient when there are many taps.

```
use rustradio::{Complex, Float};
use rustradio::graph::Graph;
use rustradio::stream::StreamType;
use rustradio::fir::low_pass_complex;
use rustradio::blocks::{ConstantSource, FftFilter, NullSink};

let mut graph = Graph::new();

// Create taps for a 100kHz low pass filter with 1kHz transition
// width.
let samp_rate: Float = 1_000_000.0;
let taps = low_pass_complex(samp_rate, 100_000.0, 1000.0);

// Set up dummy source and sink.
let src = graph.add(Box::new(ConstantSource::new(Complex::new(0.0,0.0))));
let sink = graph.add(Box::new(NullSink::<Complex>::new()));

// Create and connect fft.
let fft = graph.add(Box::new(FftFilter::new(&taps)));
graph.connect(StreamType::new_complex(), src, 0, fft, 0);
graph.connect(StreamType::new_complex(), fft, 0, sink, 0);
```
*/
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
        let o: Streamp<Complex> = w.get(0);
        loop {
            // Read so that self.buf contains exactly self.nsamples samples.
            //
            // Yes, this part is weird. It evolved into this, but any
            // cleanup I do to the next few lines just made it slower,
            // even though I removed needless logic.
            //
            // E.g.:
            // * self.buf.len() is *always* empty here, so that's a
            //   needless subtraction.
            // * We break if input available less than nsamples, so
            //   why not just compare that?
            // * Why do we even have self.buf? It's cleared on every round.
            //   (well, that means no heap allocation, sure)
            //
            // Why are these things not fixed: Because then it's
            // slower, for some reason. At least as of 2023-10-07, on
            // amd64, with Rust 1.7.1.
            let add = std::cmp::min(input.borrow().available(), self.nsamples - self.buf.len());
            if add < self.nsamples {
                break;
            }
            self.buf.extend(input.borrow().iter().take(add));
            input.borrow_mut().consume(add);

            // Run FFT.
            self.buf.resize(self.fft_size, Complex::default());
            self.fft.process(&mut self.buf);

            // Filter by array multiplication.
            //
            // TODO: check if this can be done faster by using volk.
            let mut filtered: Vec<Complex> = self
                .buf
                .iter()
                .zip(self.taps_fft.iter())
                .map(|(x, y)| x * y)
                .collect::<Vec<Complex>>();

            // IFFT back to the time domain.
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

            // Clear buffer. Per above performance comment.
            self.buf.clear();
        }
        Ok(BlockRet::Ok)
    }
}

/// FFT filter for float values.
///
/// Works just like [FftFilter], but for Float input, output, and taps.
///
/// In fact, the current implementation of FftFilterFloat is just
/// FftFilter hiding under a trenchcoat. Counter intuitively
/// therefore, this Float version of the FftFilter has a little worse
/// performance than the Complex filter.
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
        // Convert input to Complex.
        let input: Vec<Complex> = r
            .get(0)
            .borrow()
            .iter()
            .copied()
            .map(|f| Complex::new(f, 0.0))
            .collect();

        // Set up input and output streams.
        let mut is = InputStreams::new();
        let mut os = OutputStreams::new();
        is.add_stream(StreamType::from_complex(&input));
        os.add_stream(StreamType::new_complex());

        // Run Complex FftFilter.
        let ret = self.complex.work(&mut is, &mut os)?;

        // Replicate stream consume on the Float streams.
        r.get_streamtype(0).consume(input.len() - is.available(0));

        // Replicate stream write.
        //
        // You'd think calling .write() with the iterator coming out
        // of .map would be faster, but you'd be wrong.
        // (as of 2023-10-07, Rust 1.71.1, amd64)
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
    use crate::blocks::SignalSourceComplex;
    use crate::fir::low_pass_complex;

    #[test]
    fn filter_a_signal() -> Result<()> {
        // Set up parameters.
        let samp_rate = 8_000.0;
        let signal = 3000.0;
        let amplitude = 1.0;
        let cutoff = 1000.0;
        let twidth = 100.0;

        // Create blocks.
        let mut src = SignalSourceComplex::new(samp_rate, signal, amplitude);
        let taps = low_pass_complex(samp_rate, cutoff, twidth);
        let mut fft = FftFilter::new(&taps);

        // Generate a bunch of samples from signal generator.
        let out;
        {
            let mut is = InputStreams::new();
            let mut os = OutputStreams::new();
            is.add_stream(StreamType::new_disconnected());
            os.add_stream(StreamType::new_complex());
            src.work(&mut is, &mut os)?;
            out = os.get(0).borrow().iter().copied().collect::<Vec<Complex>>();
            // write_vec("bleh.txt", &out)?;
            let m = out
                .iter()
                .map(|x| x.norm_sqr().sqrt())
                .max_by(|a, b| a.total_cmp(b))
                .unwrap();
            assert!((0.999..1.001).contains(&m));
        }

        // Filter the stream.
        {
            let mut is = InputStreams::new();
            is.add_stream(StreamType::from_complex(&out));

            let mut os = OutputStreams::new();
            os.add_stream(StreamType::new_complex());

            fft.work(&mut is, &mut os)?;
            let out = os
                .get(0)
                .borrow()
                .iter()
                .skip(taps.len()) // I get garbage in the beginning.
                .copied()
                .collect::<Vec<Complex>>();
            assert!(
                out.len() > samp_rate as usize,
                "need at least 1s of data for real test. Got {}",
                out.len()
            );
            // write_vec("bleh.txt", &out)?;

            let m = out
                .iter()
                .map(|x| x.norm_sqr().sqrt())
                .max_by(|a, b| a.total_cmp(b))
                .unwrap();
            assert!(
                (0.0..0.0002).contains(&m),
                "Signal insufficiently suppressed. Got magnitude {}",
                m
            );
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn write_vec(filename: &str, v: &[Complex]) -> Result<()> {
        use std::io::BufWriter;
        use std::io::Write;
        let mut f = BufWriter::new(std::fs::File::create(filename)?);
        for s in v {
            f.write_all(&format!("{} {}\n", s.re, s.im).as_bytes())?;
        }
        Ok(())
    }
}
