//! FFT filter. Like a FIR filter, but more efficient when there are many taps.
//!
//! Backend can be the rustfft crate, or FFTW, if the `fftw` feature is enabled.
//! FFTW is a little bit faster, but requires the system library as a
//! dependency.
//!
//! ## Further reading:
//! * <https://en.wikipedia.org/wiki/Fast_Fourier_transform>
//! * <https://en.wikipedia.org/wiki/Overlap%E2%80%93add_method>
use crate::Result;
use log::trace;

use crate::block::{Block, BlockRet};
use crate::fir::AlgebraicOps;
use crate::stream::{ReadStream, Tag, WriteStream};
use crate::{Complex, Float};

/// FFT engine.
///
/// This can be either RustFFT, a pure Rust dependency, or FFTW, an external
/// standard implementation written in C.
///
/// FFTW is a little bit faster.
pub trait Engine: Send {
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

    /// FFT `Engine` for FFTW.
    pub struct FftwEngine {
        tap_len: usize,
        taps_fft: Vec<Complex>,
        fft: C2CPlan32,
        ifft: C2CPlan32,
    }

    impl FftwEngine {
        /// Create new FFTW engine, given taps.
        #[must_use]
        pub fn new<T: Into<Vec<Complex>>>(taps: T) -> Self {
            let mut taps_fft = taps.into();
            assert!(!taps_fft.is_empty());
            let tap_len = taps_fft.len();
            let fft_size = calc_fft_size(taps_fft.len());
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
            taps_fft.resize(fft_size, Complex::default());
            let mut tmp = taps_fft.clone();
            fft.c2c(&mut tmp, &mut taps_fft).unwrap();

            // Normalization is actually the square root of this
            // expression, but since we'll do two FFTs we can just skip
            // the square root here and do it just once here in setup.
            {
                let f = 1.0 / taps_fft.len() as Float;
                for s in &mut taps_fft {
                    *s *= f;
                }
            }
            Self {
                fft,
                ifft,
                taps_fft,
                tap_len,
            }
        }
    }

    impl Engine for FftwEngine {
        fn run(&mut self, i: &mut [Complex]) {
            use std::mem::MaybeUninit;

            let fft_size = self.taps_fft.len();

            // Ugly option: create un-initialized.
            let mut tmp: Vec<MaybeUninit<Complex>> = Vec::with_capacity(fft_size);
            // SAFETy:
            // It'll be filled.
            let mut tmp = unsafe {
                tmp.set_len(fft_size);
                std::mem::transmute::<Vec<MaybeUninit<Complex>>, Vec<Complex>>(tmp)
            };

            // Safer option: Create zeroed.
            // let mut tmp: Vec<Complex> = vec![Complex::default(); fft_size];

            // TODO: can we find a way to do this in-place?
            self.fft.c2c(i, &mut tmp).unwrap();
            sum_vec(&mut tmp, &self.taps_fft);
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
    /// FFT `Engine` using crate `rustfft`.
    pub struct RustFftEngine {
        tap_len: usize,
        taps_fft: Vec<Complex>,
        fft: Arc<dyn rustfft::Fft<Float>>,
        ifft: Arc<dyn rustfft::Fft<Float>>,
        scratch: Vec<Complex>,
    }
    impl RustFftEngine {
        /// Create new rustfft engine, given taps.
        #[must_use]
        pub fn new<T: Into<Vec<Complex>>>(taps: T) -> Self {
            let taps = taps.into();
            assert!(!taps.is_empty());
            let fft_size = calc_fft_size(taps.len());
            let mut planner = rustfft::FftPlanner::new();
            let fft = planner.plan_fft_forward(fft_size);
            let ifft = planner.plan_fft_inverse(fft_size);
            let mut scratch = vec![
                Complex::default();
                fft.get_inplace_scratch_len()
                    .max(ifft.get_inplace_scratch_len())
            ];
            let mut taps_fft = taps.clone();
            taps_fft.resize(fft_size, Complex::default());
            fft.process_with_scratch(&mut taps_fft, &mut scratch);
            // Normalization is actually the square root of this
            // expression, but since we'll do two FFTs we can just skip
            // the square root here and do it just once here in setup.
            {
                let f = 1.0 / taps_fft.len() as Float;
                for s in &mut taps_fft {
                    *s *= f;
                }
            }
            Self {
                fft,
                ifft,
                scratch,
                taps_fft,
                tap_len: taps.len(),
            }
        }
    }
    impl Engine for RustFftEngine {
        fn run(&mut self, i: &mut [Complex]) {
            self.fft.process_with_scratch(i, &mut self.scratch);
            sum_vec(i, &self.taps_fft);
            self.ifft.process_with_scratch(i, &mut self.scratch);
        }
        fn tap_len(&self) -> usize {
            self.tap_len
        }
    }
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
/// let taps = low_pass_complex(samp_rate, 100_000.0, 1000.0, rustradio::window::WindowType::Hamming);
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
pub struct FftFilter<T: Engine> {
    buf: Vec<Complex>,
    tags: Vec<Tag>,
    nsamples: usize,
    fft_size: usize,
    tail: Vec<Complex>,
    engine: T,
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
    pub fn new<T: Into<Vec<Complex>>>(
        src: ReadStream<Complex>,
        taps: T,
    ) -> (Self, ReadStream<Complex>) {
        trace!("FftFilter: defaulting to FFTW");
        let engine = rr_fftw::FftwEngine::new(taps);
        Self::new_engine(src, engine)
    }
}

#[cfg(not(feature = "fftw"))]
impl FftFilter<rr_rustfft::RustFftEngine> {
    /// Create a new `FftFilter` block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    pub fn new<T: Into<Vec<Complex>>>(
        src: ReadStream<Complex>,
        taps: T,
    ) -> (Self, ReadStream<Complex>) {
        trace!("FftFilter: defaulting to RustFFT");
        let engine = rr_rustfft::RustFftEngine::new(taps);
        Self::new_engine(src, engine)
    }
}
impl<T: Engine> FftFilter<T> {
    /// Create new `FftFilter`, given an engine.
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
                tags: Vec::new(),
                nsamples,
            },
            dr,
        )
    }
}

#[inline]
fn sum_vec(left: &mut [Complex], right: &[Complex]) {
    #[cfg(feature = "volk")]
    volk::volk_32fc_x2_multiply_32fc_inplace(left, &right);
    #[cfg(not(feature = "volk"))]
    left.iter_mut()
        .zip(right.iter())
        .for_each(|(x, y)| *x = x.algebraic_mul(*y));
}

impl<T: Engine> Block for FftFilter<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut output = self.dst.write_buf()?;
        if output.len() < self.nsamples {
            return Ok(BlockRet::WaitForStream(&self.dst, self.nsamples));
        }
        // Keep one snapshot for all FFT rounds that fit. In particular, WASM
        // read_buf() copies the unread window, so checking it out per round
        // would repeatedly copy the unprocessed suffix.
        let (input, tags) = self.src.read_buf()?;
        let mut tags = tags.into_iter().peekable();
        let mut output_tags = Vec::new();
        let mut consumed = 0;
        let mut produced = 0;
        let output_len = output.len();
        let wait_for_output;
        loop {
            if output_len - produced < self.nsamples {
                wait_for_output = true;
                break;
            }
            let add = (input.len() - consumed).min(self.nsamples - self.buf.len());
            let tag_offset = self.buf.len();
            self.buf
                .extend_from_slice(&input.slice()[consumed..consumed + add]);
            // Tags are ordered. Move each consumed tag exactly once, retaining
            // tags for an incomplete round until its output is ready.
            while tags.peek().is_some_and(|tag| tag.pos() < consumed + add) {
                let mut tag = tags.next().unwrap();
                tag.set_pos(tag.pos() - consumed + tag_offset);
                self.tags.push(tag);
            }
            consumed += add;
            if self.buf.len() < self.nsamples {
                wait_for_output = false;
                break;
            }

            self.buf.resize(self.fft_size, Complex::default());
            self.engine.run(&mut self.buf);

            // Add overlapping tail.
            for (i, t) in self.tail.iter().enumerate() {
                self.buf[i] = self.buf[i].algebraic_add(*t);
            }
            output.slice()[produced..produced + self.nsamples]
                .copy_from_slice(&self.buf[..self.nsamples]);
            output_tags.extend(self.tags.drain(..).map(|mut tag| {
                tag.set_pos(tag.pos() + produced);
                tag
            }));
            produced += self.nsamples;

            self.tail
                .copy_from_slice(&self.buf[self.nsamples..self.fft_size]);
            self.buf.clear();
        }
        input.consume(consumed);
        output.produce(produced, &output_tags);
        Ok(if wait_for_output {
            BlockRet::WaitForStream(&self.dst, self.nsamples)
        } else {
            BlockRet::WaitForStream(&self.src, self.nsamples - self.buf.len())
        })
    }
}

/// FFT filter for float values.
///
/// Works just like [`FftFilter`], but for Float input, output, and taps.
///
/// In fact, the current implementation of `FftFilterFloat` is just
/// `FftFilter` hiding under a trenchcoat. Counter intuitively
/// therefore, this Float version of the `FftFilter` has a little worse
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
        let engine = rr_fftw::FftwEngine::new(taps);
        Self::new_engine(src, engine)
    }
}

#[cfg(not(feature = "fftw"))]
impl FftFilterFloat<rr_rustfft::RustFftEngine> {
    /// Create a new `FftFilterFloat` block, selecting the best FFT engine.
    ///
    /// "Best" is assumed to be FFTW, if the `fftw` feature is enabled.
    /// Otherwise it's RustFFT.
    #[must_use]
    pub fn new(src: ReadStream<Float>, taps: &[Float]) -> (Self, ReadStream<Float>) {
        let taps: Vec<_> = taps.iter().map(|&f| Complex::new(f, 0.0)).collect();
        let engine = rr_rustfft::RustFftEngine::new(taps);
        Self::new_engine(src, engine)
    }
}

impl<T: Engine> FftFilterFloat<T> {
    /// Create a new `FftFilterFloat` block.
    ///
    /// Use `new()` if to make your application code engine agnostic.
    #[must_use]
    pub fn new_engine(src: ReadStream<Float>, engine: T) -> (Self, ReadStream<Float>) {
        use crate::stream::StreamWait;
        let (inner_in, r) = crate::stream::new_stream();
        assert_eq!(inner_in.id(), r.id(), "{}", inner_in.id() - r.id());
        let (complex, inner_out) = FftFilter::new_engine(r, engine);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                complex,
                src,
                dst,
                inner_in,
                inner_out,
            },
            dr,
        )
    }
}

impl<T: Engine> Block for FftFilterFloat<T> {
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
            let tags = tags
                .into_iter()
                .filter(|tag| tag.pos() < n)
                .collect::<Vec<_>>();
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
            if n == 0 && !inner_from.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let o = outer_to.slice();
            for (i, samp) in inner_from.iter().take(n).enumerate() {
                o[i] = samp.re;
            }
            let tags = tags
                .into_iter()
                .filter(|tag| tag.pos() < n)
                .collect::<Vec<_>>();
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
    use crate::blocks::{Head, SignalSourceComplex, VectorSource};
    use crate::fir::low_pass_complex;
    use crate::stream::TagValue;
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
        let (mut head, o) = Head::new(o, samp_rate as u64);
        let taps = low_pass_complex(samp_rate, cutoff, twidth, WindowType::Hamming);
        let taps_len = taps.len();
        let (mut fft, out) = FftFilter::new(o, taps);

        // Generate a bunch of samples from signal generator.
        let mut total = 0;
        loop {
            src.work()?;
            head.work()?;
            // Filter the stream.
            fft.work()?;
            let (out, tags) = out.read_buf()?;
            let out = out
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
            assert_eq!(tags, &[]);
            if total >= samp_rate as usize {
                break;
            }
        }
        Ok(())
    }

    #[test]
    fn tag_propagation() -> Result<()> {
        // Create blocks.
        let (mut src, o) = VectorSource::builder(vec![Complex::default(); 1024])
            .repeat(crate::Repeat::finite(2))
            .build()?;
        let (mut fft, out) = FftFilter::new(o, [Complex::default()]);
        src.work()?;
        src.work()?;
        fft.work()?;
        let (out, tags) = out.read_buf()?;
        assert_eq!(
            tags,
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                Tag::new(1024, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(1024, "VectorSource::repeat", TagValue::U64(1)),
            ]
        );
        assert_eq!(out.len(), 2048);
        Ok(())
    }

    #[test]
    fn batched_rounds_preserve_partial_input_and_tags() -> Result<()> {
        use crate::stream::{StreamWait, new_stream};
        let samples: Vec<_> = (0..20)
            .map(|i| Complex::new(i as Float - 7.0, i as Float * 0.25))
            .collect();
        let taps = [
            Complex::new(0.5, 0.25),
            Complex::new(-0.25, 0.5),
            Complex::new(0.125, -0.25),
        ];
        let (writer, reader) = new_stream();
        let input_id = reader.id();
        let (mut filter, out) = FftFilter::new(reader, taps);
        assert_eq!(filter.nsamples, 5);
        let all_tags = [0, 2, 3, 4, 5, 10, 15, 16, 17, 19]
            .map(|pos| Tag::new(pos, "sample", TagValue::U64(pos as u64)));
        let mut actual = Vec::new();
        let mut actual_tags = Vec::new();
        // First retain an incomplete round; then run three rounds and retain
        // another partial round; finally complete it in a subsequent call.
        for (start, end, expected_output, need) in [(0, 3, 0, 2), (3, 17, 15, 3), (17, 20, 5, 5)] {
            let tags: Vec<_> = all_tags
                .iter()
                .filter(|tag| (start..end).contains(&tag.pos()))
                .cloned()
                .map(|mut tag| {
                    tag.set_pos(tag.pos() - start);
                    tag
                })
                .collect();
            let mut window = writer.write_buf()?;
            window.fill_from_slice(&samples[start..end]);
            window.produce(end - start, &tags);
            assert!(matches!(filter.work()?, BlockRet::WaitForStream(stream, n)
                if stream.id() == input_id && n == need));
            let (window, tags) = out.read_buf()?;
            assert_eq!(window.len(), expected_output);
            actual_tags.extend(tags.into_iter().map(|mut tag| {
                tag.set_pos(tag.pos() + actual.len());
                tag
            }));
            actual.extend_from_slice(window.slice());
            window.consume(expected_output);
        }
        assert_eq!(actual_tags, all_tags);
        for (i, value) in actual.iter().enumerate() {
            let expected: Complex = taps
                .iter()
                .enumerate()
                .filter(|(j, _)| *j <= i)
                .map(|(j, tap)| samples[i - j] * tap)
                .sum();
            assert!(
                (*value - expected).norm() < 0.0001,
                "sample {i}: {value} != {expected}"
            );
        }
        Ok(())
    }

    #[test]
    fn batched_rounds_respect_output_backpressure() -> Result<()> {
        use crate::stream::{StreamWait, new_stream};
        let samples: Vec<_> = (0..17).map(|i| Complex::new(i as Float, 0.0)).collect();
        let (writer, reader) = new_stream();
        let input_id = reader.id();
        let (mut filter, out) = FftFilter::new(
            reader,
            [
                Complex::new(1.0, 0.0),
                Complex::default(),
                Complex::default(),
            ],
        );
        let output_id = out.id();
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&samples);
        window.produce(
            samples.len(),
            &[
                Tag::new(9, "before", TagValue::Bool(true)),
                Tag::new(10, "after", TagValue::Bool(true)),
                Tag::new(16, "partial", TagValue::Bool(true)),
            ],
        );
        let mut window = filter.dst.write_buf()?;
        let prefix = window.len() - 12;
        window.slice()[..prefix].fill(Complex::default());
        window.produce(prefix, &[]);
        assert!(matches!(filter.work()?, BlockRet::WaitForStream(stream, 5)
            if stream.id() == output_id));
        assert!(filter.buf.is_empty());
        let (window, tags) = filter.src.read_buf()?;
        assert_eq!(window.slice(), &samples[10..]);
        assert_eq!(tags[0].pos(), 0);
        drop(window);
        let (window, tags) = out.read_buf()?;
        assert_eq!(window.len(), prefix + 10);
        for (actual, expected) in window.slice()[prefix..].iter().zip(&samples[..10]) {
            assert!((*actual - *expected).norm() < 0.0001);
        }
        assert_eq!(tags, [Tag::new(prefix + 9, "before", TagValue::Bool(true))]);
        window.consume(prefix + 10);
        assert!(matches!(filter.work()?, BlockRet::WaitForStream(stream, 3)
            if stream.id() == input_id));
        let (window, tags) = out.read_buf()?;
        assert_eq!(window.len(), 5);
        assert_eq!(tags, [Tag::new(0, "after", TagValue::Bool(true))]);
        assert_eq!(filter.buf, samples[15..]);
        assert_eq!(filter.tags, [Tag::new(1, "partial", TagValue::Bool(true))]);
        Ok(())
    }

    #[test]
    fn work_uses_one_input_snapshot() -> Result<()> {
        use crate::stream::new_stream;
        // Simulate an upstream block producing while this batch is running.
        // The newly arrived data must remain for the next checkout/work call.
        struct ProducingEngine {
            writer: WriteStream<Complex>,
            produced: bool,
        }
        impl Engine for ProducingEngine {
            fn tap_len(&self) -> usize {
                3
            }
            fn run(&mut self, _: &mut [Complex]) {
                if !self.produced {
                    let mut window = self.writer.write_buf().unwrap();
                    window.slice()[..5].fill(Complex::default());
                    window.produce(5, &[]);
                    self.produced = true;
                }
            }
        }
        let (writer, reader) = new_stream();
        let mut window = writer.write_buf()?;
        window.slice()[..5].fill(Complex::default());
        window.produce(5, &[]);
        let (mut filter, out) = FftFilter::new_engine(
            reader,
            ProducingEngine {
                writer,
                produced: false,
            },
        );
        filter.work()?;
        let (window, _) = out.read_buf()?;
        assert_eq!(window.len(), 5);
        window.consume(5);
        filter.work()?;
        let (window, _) = out.read_buf()?;
        assert_eq!(window.len(), 5);
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
