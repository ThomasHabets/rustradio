//! FFT filter. Like a FIR filter, but more efficient when there are many taps.
//!
//! ## Further reading:
//! * <https://en.wikipedia.org/wiki/Fast_Fourier_transform>
//! * <https://en.wikipedia.org/wiki/Overlap%E2%80%93add_method>
use log::trace;
use std::sync::Arc;

use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

#[must_use]
const fn calc_fft_size(from: usize) -> usize {
    let mut n = 1;
    while n < from {
        n <<= 1;
    }
    2 * n
}

/// FFT filter. Like a FIR filter, but more efficient when there are many taps.
/// ```
/// use rustradio::{Complex, Float};
/// use rustradio::graph::{Graph, GraphRunner};
/// use rustradio::fir::low_pass_complex;
/// use rustradio::blocks::{ConstantSource, FftFilter, NullSink};
///
/// let mut graph = Graph::new();
///
/// // Create taps for a 100kHz low pass filter with 1kHz transition
/// // width.
/// let samp_rate: Float = 1_000_000.0;
/// let taps = low_pass_complex(samp_rate, 100_000.0, 1000.0, &rustradio::window::WindowType::Hamming);
///
/// // Set up dummy source and sink.
/// let (src, src_out) = ConstantSource::new(Complex::new(0.0,0.0));
///
/// // Create and connect fft.
/// let (fft, fft_out) = FftFilter::new(src_out, taps);
///
/// // Set up dummy sink.
/// let sink = NullSink::new(fft_out);
/// ```
///
/// ## Further reading:
/// * <https://en.wikipedia.org/wiki/Fast_Fourier_transform>
/// * <https://en.wikipedia.org/wiki/Overlap%E2%80%93add_method>
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FftFilter {
    buf: Vec<Complex>,

    // Number of samples per FFT batch.
    nsamples: usize,

    // FFT size.
    fft_size: usize,

    // Temporare storage between FFT batches.
    tail: Vec<Complex>,

    // FFT sized vector to filter with in the frequency domain.
    taps_fft: Vec<Complex>,

    // FFT Engines.
    fft: Arc<dyn rustfft::Fft<Float>>,
    ifft: Arc<dyn rustfft::Fft<Float>>,

    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl FftFilter {
    /// Create a new FftFilter block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new<T: Into<Vec<Complex>>>(
        src: ReadStream<Complex>,
        taps: T,
    ) -> (Self, ReadStream<Complex>) {
        trace!("FftFilter: defaulting to RustFFT");

        // Get taps & calculate FFT size.
        let mut taps = taps.into();
        let tap_len = taps.len();
        let fft_size = calc_fft_size(taps.len());

        // Create FFT plan.
        let mut planner = rustfft::FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let ifft = planner.plan_fft_inverse(fft_size);

        // Create filter.
        taps.resize(fft_size, Complex::default());
        fft.process(&mut taps);
        // Normalization is actually the square root of this
        // expression, but since we'll do two FFTs we can just skip
        // the square root here and do it just once here in setup.
        {
            let f = 1.0 / taps.len() as Float;
            taps.iter_mut().for_each(|s: &mut Complex| *s *= f);
        }

        // Set up FFT / batch size.
        let nsamples = fft_size - tap_len;

        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                fft_size,
                tail: vec![Complex::default(); tap_len],
                fft,
                ifft,
                taps_fft: taps,
                buf: Vec::with_capacity(fft_size),
                nsamples,
            },
            dr,
        )
    }
}

fn sum_vec(left: &mut [Complex], right: &[Complex]) {
    left.iter_mut().zip(right.iter()).for_each(|(x, y)| *x *= y)
}

impl Block for FftFilter {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        // TODO: multithread this.
        loop {
            let mut o = self.dst.write_buf()?;
            if self.nsamples > o.len() {
                /*
                trace!(
                    "FftFilter: Need {} output space, only have {}",
                    self.nsamples,
                    o.len()
                );
                */
                return Ok(BlockRet::WaitForStream(&self.dst, self.nsamples));
            }
            let (input, tags) = self.src.read_buf()?;
            // Read so that self.buf contains exactly self.nsamples samples.
            let add = std::cmp::min(input.len(), self.nsamples - self.buf.len());
            self.buf.extend(input.iter().take(add).copied());
            input.consume(add);
            if self.buf.len() < self.nsamples {
                /*
                trace!(
                    "FftFilter: Need {} input samples, only have {}, {add}",
                    self.nsamples,
                    self.buf.len()
                );
                */
                return Ok(BlockRet::WaitForStream(
                    &self.src,
                    self.nsamples - self.buf.len(),
                ));
            }
            //trace!("FftFilter: ok, running");
            debug_assert_eq!(self.buf.len(), self.nsamples);

            // Run FFT.
            self.buf.resize(self.fft_size, Complex::default());
            self.fft.process(&mut self.buf);
            sum_vec(&mut self.buf, &self.taps_fft);
            self.ifft.process(&mut self.buf);

            // Add overlapping tail.
            for (i, t) in self.tail.iter().enumerate() {
                self.buf[i] += t;
            }

            // Output.
            // TODO: needless copy?
            o.fill_from_slice(&self.buf[..self.nsamples]);
            o.produce(self.nsamples, &tags);

            // Stash tail.
            for i in 0..self.tail.len() {
                self.tail[i] = self.buf[self.nsamples + i];
            }

            // Clear buffer. Per above performance comment.
            self.buf.clear();
        }
    }
}

/// FFT filter for float values.
///
/// Works just like [`FftFilter`], but for Float input, output, and taps.
///
/// In fact, the current implementation of FftFilterFloat is just
/// FftFilter hiding under a trenchcoat. Counter intuitively
/// therefore, this Float version of the FftFilter has a little worse
/// performance than the Complex filter.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FftFilterFloat {
    complex: FftFilter,
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
    inner_in: WriteStream<Complex>,
    inner_out: ReadStream<Complex>,
}

impl FftFilterFloat {
    /// Create a new FftFilterFloat block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new(src: ReadStream<Float>, taps: &[Float]) -> (Self, ReadStream<Float>) {
        let taps: Vec<_> = taps.iter().map(|&f| Complex::new(f, 0.0)).collect();
        use crate::stream::StreamWait;
        let (inner_in, r) = crate::stream::new_stream();
        assert_eq!(inner_in.id(), r.id(), "{}", inner_in.id() - r.id());
        let (complex, inner_out) = FftFilter::new(r, taps);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                complex,
                inner_in,
                inner_out,
            },
            dr,
        )
    }
}

impl Block for FftFilterFloat {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        // Convert input to Complex.
        {
            let (outer_in, tags) = self.src.read_buf()?;
            let mut inner_to = self.inner_in.write_buf()?;
            let n = std::cmp::min(outer_in.len(), inner_to.len());
            let o = inner_to.slice();
            for (i, samp) in outer_in.iter().take(n).enumerate() {
                o[i] = Complex::new(*samp, 0.0);
            }
            inner_to.produce(n, &tags);
            outer_in.consume(n);
        }

        // Run Complex FftFilter.
        // TODO: if fft work function fails, for some reason, then samples are
        // lost.
        let ret = self.complex.work()?;

        // Replicate stream write.
        {
            let (inner_from, tags) = self.inner_out.read_buf()?;
            let mut outer_to = self.dst.write_buf()?;
            let n = std::cmp::min(inner_from.len(), outer_to.len());
            let o = outer_to.slice();
            for (i, samp) in inner_from.iter().take(n).enumerate() {
                o[i] = samp.re;
            }
            inner_from.consume(n);
            outer_to.produce(n, &tags);
        }

        // Replace the inner stream wait with an outer stream wait.

        Ok(match ret {
            BlockRet::WaitForStream(stream, need) => {
                use crate::stream::StreamWait;
                match stream.id() {
                    v if v == self.inner_in.id() => BlockRet::WaitForStream(&self.src, need),
                    v if v == self.inner_out.id() => BlockRet::WaitForStream(&self.dst, need),
                    other => panic!(
                        "FftFilter WaitForStream({}) is neither in ({}) nor out ({})",
                        other,
                        self.inner_in.id(),
                        self.inner_out.id()
                    ),
                }
            }
            other => other,
        })
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::blocks::SignalSourceComplex;
    use crate::fir::low_pass_complex;
    use crate::window::WindowType;

    #[test]
    fn filter_a_signal() -> Result<()> {
        // Set up parameters.
        let samp_rate = 8_000.0;
        let signal = 3000.0;
        let amplitude = 1.0;
        let cutoff = 1000.0;
        let twidth = 100.0;

        // Create blocks.
        let (mut src, o) = SignalSourceComplex::new(samp_rate, signal, amplitude);
        let taps = low_pass_complex(samp_rate, cutoff, twidth, &WindowType::Hamming);
        let taps_len = taps.len();
        let (mut fft, out) = FftFilter::new(o, taps);

        // Generate a bunch of samples from signal generator.
        let mut total = 0;
        loop {
            src.work()?;
            // Filter the stream.
            fft.work()?;
            let out = out
                .read_buf()?
                .0
                .iter()
                .skip(taps_len) // I get garbage in the beginning.
                .copied()
                .collect::<Vec<Complex>>();
            // write_vec("bleh.txt", &out)?;

            total += out.len();
            let m = out
                .iter()
                .map(|x| x.norm_sqr().sqrt())
                .max_by(|a, b| a.total_cmp(b))
                .unwrap();
            assert!(
                (0.0..0.0002).contains(&m),
                "Signal insufficiently suppressed. Got magnitude {m}"
            );
            if total > samp_rate as usize {
                break;
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn write_vec(filename: &str, v: &[Complex]) -> Result<()> {
        use std::io::BufWriter;
        use std::io::Write;
        let mut f = BufWriter::new(std::fs::File::create(filename)?);
        for s in v {
            f.write_all(format!("{} {}\n", s.re, s.im).as_bytes())?;
        }
        Ok(())
    }
}
/* vim: textwidth=80
 */
