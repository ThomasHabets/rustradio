/*! FFT filter. Like a FIR filter, but more efficient when there are many taps.

```
use rustradio::{Complex, Float};
use rustradio::graph::{Graph, GraphRunner};
use rustradio::fir::low_pass_complex;
use rustradio::blocks::{ConstantSource, FftFilter, NullSink};

let mut graph = Graph::new();

// Create taps for a 100kHz low pass filter with 1kHz transition
// width.
let samp_rate: Float = 1_000_000.0;
let taps = low_pass_complex(samp_rate, 100_000.0, 1000.0, &rustradio::window::WindowType::Hamming);

// Set up dummy source and sink.
let (src, src_out) = ConstantSource::new(Complex::new(0.0,0.0));

// Create and connect fft.
let (fft, fft_out) = FftFilter::new(src_out, &taps);

// Set up dummy sink.
let sink = NullSink::new(fft_out);
```

## Further reading:
* <https://en.wikipedia.org/wiki/Fast_Fourier_transform>
* <https://en.wikipedia.org/wiki/Overlap%E2%80%93add_method>
*/
use anyhow::Result;
use log::trace;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Float};

/// FFT engine.
///
/// This can be either RustFFT, a pure Rust dependency, or FFTW, an external
/// standard implementation written in C.
///
/// FFTW is a little bit faster.
pub trait Engine {
    /// Run runs an FFT round. Input and output is always the size of the FFT.
    fn run(&mut self, i: &mut [Complex]);

    /// Return the number of taps used.
    ///
    /// This is just so that we don't have to keep track of both the engine and the original tap
    /// len.
    #[must_use]
    fn tap_len(&self) -> usize;
}

#[must_use]
fn calc_fft_size(from: usize) -> usize {
    let mut n = 1;
    while n < from {
        n <<= 1;
    }
    2 * n
}

#[cfg(feature = "fftw")]
pub mod rr_fftw {
    use super::*;
    use fftw::plan::{C2CPlan, C2CPlan32};

    pub struct FftwEngine {
        tap_len: usize,
        taps_fft: Vec<Complex>,
        fft: C2CPlan32,
        ifft: C2CPlan32,
    }

    impl FftwEngine {
        #[must_use]
        pub fn new(taps: &[Complex]) -> Self {
            let fft_size = calc_fft_size(taps.len());
            // Create FFT planners.
            let mut fft: C2CPlan32 = C2CPlan::aligned(
                &[fft_size],
                fftw::types::Sign::Forward,
                fftw::types::Flag::MEASURE,
            )
            .unwrap();
            let ifft: C2CPlan32 = C2CPlan::aligned(
                &[fft_size],
                fftw::types::Sign::Backward,
                fftw::types::Flag::MEASURE,
            )
            .unwrap();

            // Pre-FFT the taps.
            let mut taps_fft = taps.to_vec();
            taps_fft.resize(fft_size, Complex::default());
            let mut tmp = taps_fft.clone();
            fft.c2c(&mut tmp, &mut taps_fft).unwrap();

            // Normalization is actually the square root of this
            // expression, but since we'll do two FFTs we can just skip
            // the square root here and do it just once here in setup.
            {
                let f = 1.0 / taps_fft.len() as Float;
                taps_fft.iter_mut().for_each(|s: &mut Complex| *s *= f);
            }
            Self {
                fft,
                ifft,
                taps_fft,
                tap_len: taps.len(),
            }
        }
    }

    impl Engine for FftwEngine {
        fn run(&mut self, i: &mut [Complex]) {
            let fft_size = self.taps_fft.len();

            // Ugly option: create un-initialized.
            let mut tmp = Vec::with_capacity(fft_size);
            unsafe { tmp.set_len(fft_size) };

            // Safer option: Create zeroed.
            // let mut tmp: Vec<Complex> = vec![Complex::default(); fft_size];

            self.fft.c2c(i, &mut tmp).unwrap();
            sum_vec(&mut tmp, &self.taps_fft);

            // TODO: I really don't get why I can't get it to work storing directly into the output
            // slice.
            self.ifft.c2c(&mut tmp, i).unwrap();
        }
        fn tap_len(&self) -> usize {
            self.tap_len
        }
    }
}

pub mod rr_rustfft {
    use super::*;
    use std::sync::Arc;
    pub struct RustFftEngine {
        tap_len: usize,
        taps_fft: Vec<Complex>,
        fft: Arc<dyn rustfft::Fft<Float>>,
        ifft: Arc<dyn rustfft::Fft<Float>>,
    }
    impl RustFftEngine {
        #[must_use]
        pub fn new(taps: &[Complex]) -> Self {
            let fft_size = calc_fft_size(taps.len());
            let mut planner = rustfft::FftPlanner::new();
            let fft = planner.plan_fft_forward(fft_size);
            let ifft = planner.plan_fft_inverse(fft_size);
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
                fft,
                ifft,
                taps_fft,
                tap_len: taps.len(),
            }
        }
    }
    impl Engine for RustFftEngine {
        fn run(&mut self, i: &mut [Complex]) {
            self.fft.process(i);
            sum_vec(i, &self.taps_fft);
            self.ifft.process(i);
        }
        fn tap_len(&self) -> usize {
            self.tap_len
        }
    }
}

/// FFT filter. Like a FIR filter, but more efficient when there are many taps.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FftFilter<T: Engine> {
    buf: Vec<Complex>,
    nsamples: usize,
    fft_size: usize,
    tail: Vec<Complex>,
    engine: T,
    //engine: rr_rustfft::RustFftEngine,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

#[cfg(feature = "fftw")]
impl FftFilter<rr_fftw::FftwEngine> {
    /// Create a new FftFilter block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new(src: ReadStream<Complex>, taps: &[Complex]) -> (Self, ReadStream<Complex>) {
        trace!("FftFilter: defaulting to FFTW");
        let engine = rr_fftw::FftwEngine::new(taps);
        Self::new_engine(src, engine)
    }
}

#[cfg(not(feature = "fftw"))]
impl FftFilter<rr_rustfft::RustFftEngine> {
    /// Create a new FftFilter block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new(src: ReadStream<Complex>, taps: &[Complex]) -> (Self, ReadStream<Complex>) {
        trace!("FftFilter: defaulting to RustFFT");
        let engine = rr_rustfft::RustFftEngine::new(taps);
        Self::new_engine(src, engine)
    }
}
impl<T: Engine> FftFilter<T> {
    /// Create new FftFilter, given filter taps.
    #[must_use]
    pub fn new_engine(src: ReadStream<Complex>, engine: T) -> (Self, ReadStream<Complex>) {
        // Set up FFT / batch size.
        let fft_size = calc_fft_size(engine.tap_len());
        let nsamples = fft_size - engine.tap_len();

        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                fft_size,
                tail: vec![Complex::default(); engine.tap_len()],
                engine,
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

impl<T: Engine> Block for FftFilter<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        // TODO: multithread this.
        let mut produced = false;
        loop {
            let (input, tags) = self.src.read_buf()?;
            let mut o = self.dst.write_buf()?;

            if self.nsamples > o.len() {
                trace!(
                    "FftFilter: Need {} output space, only have {}",
                    self.nsamples,
                    o.len()
                );
                break;
            }
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
            let add = std::cmp::min(input.len(), self.nsamples - self.buf.len());
            if add < self.nsamples {
                break;
            }
            self.buf.extend(input.iter().take(add).copied());
            input.consume(add);

            // Run FFT.
            self.buf.resize(self.fft_size, Complex::default());
            self.engine.run(&mut self.buf);

            // Add overlapping tail.
            for (i, t) in self.tail.iter().enumerate() {
                self.buf[i] += t;
            }

            // Output.
            // TODO: needless copy?
            o.fill_from_slice(&self.buf[..self.nsamples]);
            o.produce(self.nsamples, &tags);
            produced = true;

            // Stash tail.
            for i in 0..self.tail.len() {
                self.tail[i] = self.buf[self.nsamples + i];
            }

            // Clear buffer. Per above performance comment.
            self.buf.clear();
        }
        if produced {
            Ok(BlockRet::Ok)
        } else {
            Ok(BlockRet::Noop)
        }
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
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FftFilterFloat<T: Engine> {
    complex: FftFilter<T>,
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
    inner_in: WriteStream<Complex>,
    inner_out: ReadStream<Complex>,
}

#[cfg(feature = "fftw")]
impl FftFilterFloat<rr_fftw::FftwEngine> {
    /// Create a new FftFilterFloat block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new(src: ReadStream<Float>, taps: &[Float]) -> (Self, ReadStream<Float>) {
        let taps: Vec<_> = taps.iter().map(|&f| Complex::new(f, 0.0)).collect();
        let engine = rr_fftw::FftwEngine::new(&taps);
        Self::new_engine(src, engine)
    }
}

#[cfg(not(feature = "fftw"))]
impl FftFilterFloat<rr_rustfft::RustFftEngine> {
    /// Create a new FftFilterFloat block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new(src: ReadStream<Float>, taps: &[Float]) -> (Self, ReadStream<Float>) {
        let taps: Vec<_> = taps.iter().map(|&f| Complex::new(f, 0.0)).collect();
        let engine = rr_rustfft::RustFftEngine::new(&taps);
        Self::new_engine(src, engine)
    }
}

impl<T: Engine> FftFilterFloat<T> {
    /// Create a new FftFilterFloat block.
    ///
    /// Use `new()` if to make your application code engine agnostic.
    #[must_use]
    pub fn new_engine(src: ReadStream<Float>, engine: T) -> (Self, ReadStream<Float>) {
        let (inner_in, r) = crate::stream::new_stream();
        let (complex, inner_out) = FftFilter::new_engine(r, engine);
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

impl<T: Engine> Block for FftFilterFloat<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
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
        Ok(ret)
    }
}

#[cfg(test)]
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
        let (mut fft, out) = FftFilter::new(o, &taps);

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
                .skip(taps.len()) // I get garbage in the beginning.
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
                "Signal insufficiently suppressed. Got magnitude {}",
                m
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
