//! Finite impulse response filter.
//!
//! If using many taps, [`FftFilter`](crate::blocks::FftFilter) probably has
//! better performance.
//!
use std::borrow::Borrow;
use std::sync::{Arc, Mutex, Weak};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::window::{Window, WindowType};
use crate::{AlgebraicAdd, AlgebraicMul, Complex, Error, Float, Result, Sample};

/// Slice-based finite impulse response filter, parameterized by coefficient type.
///
/// Input samples can have a different type and need no storage or encoding
/// traits. For example, real coefficients can filter complex samples:
///
/// ```
/// use rustradio::{Complex, fir::Fir};
/// let fir = Fir::new([0.5f32, 0.5])?;
/// assert_eq!(fir.filter(&[Complex::new(2.0, 1.0), Complex::new(4.0, 3.0)]),
///            Complex::new(3.0, 2.0));
/// # Ok::<(), rustradio::Error>(())
/// ```
pub struct Fir<C> {
    taps: Box<[C]>,
}

#[cfg(all(
    target_feature = "avx",
    target_feature = "sse3",
    target_feature = "sse"
))]
#[allow(unreachable_code)]
fn sum_product_avx(vec1: &[f32], vec2: &[f32]) -> f32 {
    // SAFETY: Pointer arithmetic "should be fine". And as for instruction availability, that could
    // be checked by the macro above.
    unsafe {
        use core::arch::x86_64::*;
        assert!(vec2.len() >= vec1.len());
        let len = vec1.len() - vec1.len() % 8;

        // AVX.
        let mut sum = _mm256_setzero_ps(); // Initialize sum vector to zeros.

        for i in (0..len).step_by(8) {
            // AVX.
            let a = _mm256_loadu_ps(vec1.as_ptr().add(i));
            let b = _mm256_loadu_ps(vec2.as_ptr().add(i));

            // Multiply and accumulate.
            // AVX.
            let prod = _mm256_mul_ps(a, b);
            sum = _mm256_add_ps(sum, prod);
        }

        // Split.
        // AVX.
        let low = _mm256_extractf128_ps(sum, 0);
        let high = _mm256_extractf128_ps(sum, 1);

        // Compact step 1 => 4 floats.
        // SSE3.
        let m128 = _mm_hadd_ps(low, high);

        // Compact step 2 => 2 floats.
        // SSE3.
        let m128 = _mm_hadd_ps(m128, low);

        // Compact step 3 => 1 floats.
        // SSE3.
        let m128 = _mm_hadd_ps(m128, low);
        // SSE.
        let partial = _mm_cvtss_f32(m128);
        let skip = vec1.len() - vec1.len() % 8;
        vec1[skip..]
            .iter()
            .zip(vec2[skip..].iter())
            .fold(partial, |acc, (&f, &x)| {
                acc.algebraic_add(x.algebraic_mul(f))
            })
    }
}

impl Fir<Float> {
    /// Run filter once, creating one sample from the taps and an
    /// equal number of input samples.
    #[must_use]
    pub fn filter_float(&self, input: &[Float]) -> Float {
        #[cfg(all(target_arch = "wasm32", target_feature = "simd128",))]
        return sum_product_float_wasm(input, &self.taps);
        // AVX is faster, when available.
        #[cfg(all(
            target_feature = "avx",
            target_feature = "sse3",
            target_feature = "sse"
        ))]
        return sum_product_avx(&self.taps, input);
        // Second fastest is generic simd.
        #[cfg(feature = "simd")]
        #[allow(unreachable_code)]
        {
            use std::simd::num::SimdFloat;
            type Batch = std::simd::f32x8;

            let batch_n = 8;
            // How will this work if Float is f64?
            let partial = input
                .chunks_exact(batch_n)
                .zip(self.taps.chunks_exact(batch_n))
                .map(|(a, b)| Batch::from_slice(a) * Batch::from_slice(b))
                .fold(Batch::splat(0.0), |acc, x| acc + x)
                .reduce_sum();
            // Maybe even faster if doing a second round with f32x4.
            let skip = self.taps.len() - self.taps.len() % batch_n;
            return input[skip..]
                .iter()
                .zip(self.taps[skip..].iter())
                .fold(partial, |acc, (&f, &x)| {
                    acc.algebraic_add(x.algebraic_mul(f))
                });
        }
        #[allow(unreachable_code)]
        self.filter(input)
    }
}

impl<C> Fir<C> {
    /// Create a FIR, taking ownership of coefficients and reversing them in place.
    ///
    /// Returns an error for empty coefficients. No stream or encoding traits are
    /// needed: the kernel operates on slices, and coefficients may have a
    /// different type from the input samples.
    pub fn new(taps: impl Into<Box<[C]>>) -> Result<Self> {
        let mut taps = taps.into();
        if taps.is_empty() {
            return Err(Error::msg("FIR taps must be nonempty"));
        }
        taps.reverse();
        Ok(Self { taps })
    }

    /// Run the filter once, multiplying each input sample by its coefficient.
    ///
    /// # Panics
    ///
    /// Panics if `input` is shorter than the coefficients.
    #[must_use]
    pub fn filter<T>(&self, input: &[T]) -> T
    where
        C: Copy,
        T: Copy + AlgebraicAdd<Output = T> + AlgebraicMul<C, Output = T>,
    {
        assert!(
            input.len() >= self.taps.len(),
            "input {} < taps {}",
            input.len(),
            self.taps.len()
        );
        let mut products = input
            .iter()
            .zip(self.taps.iter())
            .map(|(&sample, &tap)| sample.algebraic_mul(tap));
        // Nonempty taps let us avoid assuming that Default is numerical zero.
        let first = products.next().expect("FIR taps are nonempty");
        products.fold(first, |acc, product| acc.algebraic_add(product))
    }

    /// Call `filter()` multiple times, across an input range.
    #[must_use]
    pub fn filter_n<T>(&self, input: &[T], deci: usize) -> Vec<T>
    where
        C: Copy,
        T: Copy + AlgebraicAdd<Output = T> + AlgebraicMul<C, Output = T>,
    {
        assert_ne!(deci, 0);
        assert!(input.len() >= self.taps.len());
        let n = input.len() - self.taps.len();
        (0..=n)
            .step_by(deci)
            .map(|i| self.filter(&input[i..]))
            .collect()
    }

    /// Like `filter_n`, but writes directly into the destination.
    pub fn filter_n_inplace<T>(&self, input: &[T], deci: usize, out: &mut [T])
    where
        C: Copy,
        T: Copy + AlgebraicAdd<Output = T> + AlgebraicMul<C, Output = T>,
    {
        assert_ne!(deci, 0);
        out.iter_mut()
            .enumerate()
            .for_each(|(i, o)| *o = self.filter(&input[(i * deci)..]));
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn horizontal_sum_f32x4(v: core::arch::wasm32::v128) -> Float {
    use core::arch::wasm32::f32x4_extract_lane;

    f32x4_extract_lane::<0>(v)
        + f32x4_extract_lane::<1>(v)
        + f32x4_extract_lane::<2>(v)
        + f32x4_extract_lane::<3>(v)
}

// As of 2026-06-27, this specialization shaves off about 20% of float FIR CPU.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn sum_product_float_wasm(input: &[Float], taps: &[Float]) -> Float {
    use core::arch::wasm32::*;

    assert!(
        input.len() >= taps.len(),
        "input {} < taps {}",
        input.len(),
        taps.len()
    );

    let len = taps.len() - taps.len() % 4;
    let mut acc = f32x4_splat(0.0);
    for n in (0..len).step_by(4) {
        // SAFETY: `len` is rounded down to a multiple of four and no larger
        // than `taps.len()`, while the caller already checked
        // `input.len() >= taps.len()`. `v128_load` is valid for unaligned wasm
        // memory, and each iteration loads exactly four f32s from both slices.
        let (input_v, taps_v) = unsafe {
            (
                v128_load(input.as_ptr().add(n).cast()),
                v128_load(taps.as_ptr().add(n).cast()),
            )
        };
        acc = f32x4_add(acc, f32x4_mul(input_v, taps_v));
    }

    input[len..]
        .iter()
        .zip(taps[len..].iter())
        .fold(horizontal_sum_f32x4(acc), |acc, (&input, &tap)| {
            acc.algebraic_add(tap.algebraic_mul(input))
        })
}

// This ended up not being faster. Leaving it here for now.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[allow(unused)]
fn sum_product_complex_wasm(input: &[Complex], taps: &[Complex]) -> Complex {
    use core::arch::wasm32::*;

    assert!(
        input.len() >= taps.len(),
        "input {} < taps {}",
        input.len(),
        taps.len()
    );
    assert_eq!(
        std::mem::size_of::<Complex>(),
        2 * std::mem::size_of::<Float>()
    );

    let len = taps.len() - taps.len() % 2;
    let input_ptr = input.as_ptr().cast::<Float>();
    let taps_ptr = taps.as_ptr().cast::<Float>();

    // Each v128 holds two Complex<f32> values as [re0, im0, re1, im1].
    // Accumulate real and imaginary terms separately, then reduce lanes once.
    let mut real_acc = f32x4_splat(0.0);
    let mut imag_acc = f32x4_splat(0.0);
    let real_signs = f32x4(1.0, -1.0, 1.0, -1.0);
    for n in (0..len).step_by(2) {
        // SAFETY: `len` is even and no larger than `taps.len()`, while the
        // caller already checked `input.len() >= taps.len()`. `v128_load` is
        // valid for unaligned wasm memory, and each iteration loads exactly two
        // complex samples from both slices.
        let (input_v, taps_v) = unsafe {
            (
                v128_load(input_ptr.add(2 * n).cast()),
                v128_load(taps_ptr.add(2 * n).cast()),
            )
        };
        let products = f32x4_mul(input_v, taps_v);
        real_acc = f32x4_add(real_acc, f32x4_mul(products, real_signs));

        let input_swapped = i32x4_shuffle::<1, 0, 3, 2>(input_v, input_v);
        imag_acc = f32x4_add(imag_acc, f32x4_mul(input_swapped, taps_v));
    }

    let mut ret = Complex::new(
        horizontal_sum_f32x4(real_acc),
        horizontal_sum_f32x4(imag_acc),
    );
    for (&input, &tap) in input[len..].iter().zip(taps[len..].iter()) {
        ret = ret.algebraic_add(tap.algebraic_mul(input));
    }
    ret
}

/// Builder for a FIR filter block.
///
/// Sample and coefficient types stay the same when translation is configured.
pub struct FirFilterBuilder<T, C = T> {
    taps: Box<[C]>,
    deci: usize,
    translation: Option<Translation<T, C>>,
}

impl<T, C> FirFilterBuilder<T, C> {
    /// Set the decimation. Defaults to one; zero is rejected by `build()`.
    #[must_use]
    pub fn deci(mut self, deci: usize) -> Self {
        self.deci = deci;
        self
    }
}

impl<T, C> FirFilterBuilder<T, C>
where
    T: Sample + AlgebraicAdd<Output = T> + AlgebraicMul<C, Output = T>,
    C: Copy + Send,
{
    /// Build a FIR with the supplied input stream.
    ///
    /// Rejects empty coefficients, zero decimation, an input window exceeding
    /// stream capacity, or invalid frequency translation settings.
    pub fn build(self, src: ReadStream<T>) -> Result<(FirFilter<T, C>, ReadStream<T>)> {
        let Self {
            taps,
            deci,
            translation,
        } = self;
        if taps.is_empty() || deci == 0 {
            return Err(Error::msg("FIR taps and decimation must be nonzero"));
        }
        let need = taps
            .len()
            .checked_add(deci - 1)
            .ok_or_else(|| Error::msg("FIR input window size overflow"))?;
        if need > src.total_size() {
            return Err(Error::msg("FIR input window exceeds stream capacity"));
        }
        if let Some(translation) = &translation
            && let Some(settings) = translation.state.requested()?
        {
            settings.validate()?;
        }
        let ntaps = taps.len();
        let fir = Fir::new(taps)?;
        let (dst, stream) = crate::stream::new_stream()?;
        Ok((
            FirFilter {
                src,
                dst,
                fir,
                ntaps,
                deci,
                translation,
            },
            stream,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TranslationSettings {
    samp_rate: Float,
    freq: Float,
}

impl TranslationSettings {
    fn validate(self) -> Result<()> {
        if !self.samp_rate.is_finite() || self.samp_rate <= 0.0 || !self.freq.is_finite() {
            return Err(Error::msg(
                "FIR translation requires a finite positive sample rate and finite frequency",
            ));
        }
        Ok(())
    }
}

type TranslationRequest = Mutex<Option<TranslationSettings>>;

/// Runtime frequency translation control for a complex [`FirFilter`].
///
/// Clones can be sent to other threads after the filter is moved into a graph.
/// Successful requests are validated and scheduled for the next output-producing
/// work call; they do not wait for processing. The latest request at that batch
/// boundary wins. Waiting for input or output space does not advance phase.
#[derive(Debug, Clone)]
pub struct FirFilterControl {
    request: Weak<TranslationRequest>,
}

impl FirFilterControl {
    /// Mix by `-freq` Hz using the supplied sample rate, preserving mixer phase.
    ///
    /// Changes apply to the next batch's input windows. Zero frequency holds
    /// the current phase; use [`Self::disable_translation`] to remove mixing.
    ///
    /// # Errors
    ///
    /// Rejects nonfinite values, a nonpositive sample rate, or a dropped filter.
    pub fn set_translation(&self, samp_rate: Float, freq: Float) -> Result<()> {
        let settings = TranslationSettings { samp_rate, freq };
        settings.validate()?;
        self.request(Some(settings))
    }

    /// Restore the original filter coefficients and reset mixer phase to zero.
    ///
    /// Applies at the next output-producing work call. Returns an error if the
    /// filter has been dropped. Re-enabling starts at zero phase at that batch's
    /// first input-window sample.
    pub fn disable_translation(&self) -> Result<()> {
        self.request(None)
    }

    fn request(&self, settings: Option<TranslationSettings>) -> Result<()> {
        let request = self
            .request
            .upgrade()
            .ok_or_else(|| Error::msg("FIR control: filter has been dropped"))?;
        *request
            .lock()
            .map_err(|_| Error::msg("FIR control lock poisoned"))? = settings;
        Ok(())
    }
}

// Erase only the complex batch entry point, so ordinary sample types need no
// translation trait. Coefficients and samples retain their public static types.
// Dispatch happens once per batch, never in the per-sample FIR loop.
type TranslateBatch<T, C> =
    fn(&mut TranslationState, &Fir<C>, &[T], usize, &mut [T]) -> Result<bool>;

struct Translation<T, C> {
    state: TranslationState,
    filter: TranslateBatch<T, C>,
}

struct TranslationState {
    request: Arc<TranslationRequest>,
    applied: Option<TranslationSettings>,
    rotated: Option<Fir<Complex>>,
    // Mixer phase at the first sample of the next input window.
    phase: num_complex::Complex<f64>,
    step: num_complex::Complex<f64>,
    until_normalize: usize,
}

impl TranslationState {
    fn requested(&self) -> Result<Option<TranslationSettings>> {
        self.request
            .lock()
            .map(|settings| *settings)
            .map_err(|_| Error::msg("FIR control lock poisoned"))
    }
}

impl<C: Copy + Into<Complex>> Translation<Complex, C> {
    fn new(settings: Option<TranslationSettings>) -> Self {
        Self {
            state: TranslationState {
                request: Arc::new(Mutex::new(settings)),
                applied: None,
                rotated: None,
                phase: num_complex::Complex::new(1.0, 0.0),
                step: num_complex::Complex::new(1.0, 0.0),
                until_normalize: 4096,
            },
            filter: filter_translated::<C>,
        }
    }
}

fn filter_translated<C: Copy + Into<Complex>>(
    state: &mut TranslationState,
    original: &Fir<C>,
    input: &[Complex],
    deci: usize,
    output: &mut [Complex],
) -> Result<bool> {
    let requested = state.requested()?;
    if requested != state.applied {
        if let Some(settings) = requested {
            let input_step = -2.0 * std::f64::consts::PI * f64::from(settings.freq)
                / f64::from(settings.samp_rate);
            let tap_step = num_complex::Complex::new(input_step.cos(), input_step.sin());
            let mut tap_phase = num_complex::Complex::new(1.0, 0.0);
            // The original kernel stores reversed coefficients. Incorporate
            // mixing relative to the start of each window, then rotate each
            // filtered output by that window's continuing mixer phase.
            let taps = original
                .taps
                .iter()
                .map(|&tap| {
                    let tap = tap
                        .into()
                        .algebraic_mul(Complex::new(tap_phase.re as Float, tap_phase.im as Float));
                    tap_phase *= tap_step;
                    tap
                })
                .collect();
            state.rotated = Some(Fir { taps });
            let output_step = input_step * deci as f64;
            state.step = num_complex::Complex::new(output_step.cos(), output_step.sin());
        } else {
            state.rotated = None;
            state.phase = num_complex::Complex::new(1.0, 0.0);
            state.step = num_complex::Complex::new(1.0, 0.0);
            state.until_normalize = 4096;
        }
        state.applied = requested;
    }
    if requested.is_none()
        || (requested.is_some_and(|settings| settings.freq == 0.0)
            && state.phase == num_complex::Complex::new(1.0, 0.0))
    {
        return Ok(false);
    }
    state
        .rotated
        .as_ref()
        .expect("active translation has prepared coefficients")
        .filter_n_inplace(input, deci, output);
    for sample in output {
        *sample = sample.algebraic_mul(Complex::new(
            state.phase.re as Float,
            state.phase.im as Float,
        ));
        state.phase *= state.step;
        state.until_normalize -= 1;
        if state.until_normalize == 0 {
            state.phase /= state.phase.norm();
            state.until_normalize = 4096;
        }
    }
    Ok(true)
}

impl<C: Copy + Into<Complex>> FirFilterBuilder<Complex, C> {
    /// Mix by `-freq` Hz while filtering complex samples.
    ///
    /// Keeps the builder and filter types unchanged, including real coefficient
    /// types. `build()` validates these settings. Use [`FirFilter::control`] for
    /// subsequent changes while the graph is running.
    #[must_use]
    pub fn translate(mut self, samp_rate: Float, freq: Float) -> Self {
        self.translation = Some(Translation::new(Some(TranslationSettings {
            samp_rate,
            freq,
        })));
        self
    }
}

/// Streaming FIR with independent sample and coefficient types.
///
/// `C` defaults to `T`. Ordinary custom samples need no frequency translation
/// implementation. Complex filters can enable, retune, and disable translation
/// without changing their type, using [`Self::control`].
///
/// ```
/// use rustradio::{Complex, fir::FirFilter, stream::new_stream};
/// let (_writer, input) = new_stream::<Complex>()?;
/// let (mut filter, _output) = FirFilter::new(input, [0.25f32, 0.5, 0.25])?;
/// let control = filter.control();
/// control.set_translation(48000.0, 1000.0)?;
/// // Keep `control` when moving `filter` into a running graph.
/// control.set_translation(48000.0, 2000.0)?;
/// control.disable_translation()?;
/// # Ok::<(), rustradio::Error>(())
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FirFilter<T: Sample, C: Copy + Send = T> {
    fir: Fir<C>,
    ntaps: usize,
    deci: usize,
    translation: Option<Translation<T, C>>,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample, C: Copy + Send> FirFilter<T, C> {
    /// Configure a FIR with the supplied coefficients.
    pub fn builder(taps: impl Into<Box<[C]>>) -> FirFilterBuilder<T, C> {
        FirFilterBuilder {
            taps: taps.into(),
            deci: 1,
            translation: None,
        }
    }

    /// Create a FIR without frequency translation.
    ///
    /// Rejects empty coefficients and windows exceeding input capacity.
    pub fn new(src: ReadStream<T>, taps: impl Into<Box<[C]>>) -> Result<(Self, ReadStream<T>)>
    where
        T: AlgebraicAdd<Output = T> + AlgebraicMul<C, Output = T>,
    {
        Self::builder(taps).build(src)
    }
}

impl<C: Copy + Send + Into<Complex>> FirFilter<Complex, C> {
    /// Obtain a cloneable translation control handle before moving into a graph.
    ///
    /// This lazily attaches control state, preserving the original coefficients.
    /// Calling it alone leaves translation disabled.
    pub fn control(&mut self) -> FirFilterControl {
        let translation = self
            .translation
            .get_or_insert_with(|| Translation::new(None));
        FirFilterControl {
            request: Arc::downgrade(&translation.state.request),
        }
    }

    /// Schedule translation at the next output-producing work call.
    ///
    /// Equivalent to [`FirFilterControl::set_translation`] when holding the block.
    pub fn set_translation(&mut self, samp_rate: Float, freq: Float) -> Result<()> {
        self.control().set_translation(samp_rate, freq)
    }

    /// Schedule disabling translation and resetting phase at the next output batch.
    pub fn disable_translation(&mut self) -> Result<()> {
        self.control().disable_translation()
    }
}

impl<T, C> Block for FirFilter<T, C>
where
    T: Sample + AlgebraicAdd<Output = T> + AlgebraicMul<C, Output = T>,
    C: Copy + Send,
{
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (input, mut tags) = self.src.read_buf()?;

        // Get number of input samples we intend to consume.
        let n = {
            // Carefully avoid underflow.
            let absolute_minimum = self.ntaps + self.deci - 1;
            if input.len() < absolute_minimum {
                return Ok(BlockRet::WaitForStream(&self.src, absolute_minimum));
            }
            self.deci * ((input.len() - self.ntaps + 1) / self.deci)
        };
        assert_ne!(n, 0);

        // To consume `n`, we may need more input samples than that.
        let need = n + self.ntaps - 1;
        assert!(input.len() >= need, "need {need}, have {}", input.len());

        // Output must have room for at least one sample.
        let mut out = self.dst.write_buf()?;
        let need_out = 1;
        if out.len() < need_out {
            return Ok(BlockRet::WaitForStream(&self.dst, need_out));
        }

        // Cap by output capacity.
        let n = std::cmp::min(n, out.len().saturating_mul(self.deci));

        // Final `n` (samples to consume) calculated. Sanity check it.
        assert_eq!(n % self.deci, 0);
        assert_ne!(n, 0, "input: {} out: {}", input.len(), out.len());

        // Run the FIR.
        let out_n = n / self.deci;
        let translated = if let Some(translation) = &mut self.translation {
            (translation.filter)(
                &mut translation.state,
                &self.fir,
                &input.slice()[..need],
                self.deci,
                &mut out.slice()[..out_n],
            )?
        } else {
            false
        };
        if !translated {
            self.fir
                .filter_n_inplace(&input.slice()[..need], self.deci, &mut out.slice()[..out_n]);
        }

        // Sanity check the generated output.
        assert!(out_n <= out.len());

        tags.retain(|tag| tag.pos() < n);
        input.consume(n);
        if self.deci == 1 {
            out.produce(out_n, tags);
        } else {
            for t in &mut tags {
                t.set_pos(t.pos() / self.deci);
            }
            out.produce(out_n, tags);
        }
        // While we could keep track of which stream is the constraining factor,
        // the code is simpler if work() is just called again, and the right
        // WaitForStream is returned above instead.
        Ok(BlockRet::Again)
    }
}

/// Create a fixed-length set of complex multiband filter taps.
///
/// Band endpoints are normalized to Nyquist and must satisfy
/// `0 <= low <= high <= 1`.
///
/// Returns an error for nonfinite frequencies or window weights, invalid
/// bands, or a zero tap count or one that differs from the window length.
pub fn multiband(bands: &[(Float, Float)], taps: usize, window: &Window) -> Result<Box<[Complex]>> {
    use rustfft::FftPlanner;

    if taps == 0 || taps != window.0.len() {
        return Err(Error::msg(
            "Multiband tap count must be nonzero and match the window length",
        ));
    }
    validate_window(window)?;
    for &(low, high) in bands {
        if !low.is_finite() || !high.is_finite() || low < 0.0 || low > high || high > 1.0 {
            return Err(Error::msg(
                "Multiband endpoints must satisfy 0 <= low <= high <= 1 and be finite",
            ));
        }
    }

    let mut ideal = vec![Complex::new(0.0, 0.0); taps].into_boxed_slice();
    let scale = (taps as Float) / 2.0;
    for (low, high) in bands {
        let a = (low * scale).floor() as usize;
        let b = (high * scale).ceil() as usize;
        for n in a..b {
            ideal[n] = Complex::new(1.0, 0.0);
            ideal[taps - n - 1] = Complex::new(1.0, 0.0);
        }
    }
    let fft_size = taps;
    let mut planner = FftPlanner::new();
    let ifft = planner.plan_fft_inverse(fft_size);
    ifft.process(&mut ideal);
    ideal.rotate_right(taps / 2);
    let scale = (fft_size as Float).sqrt();
    for (tap, weight) in ideal.iter_mut().zip(window.0.iter()) {
        *tap = *tap * *weight / Complex::new(scale, 0.0);
    }
    Ok(ideal)
}

/// Create a fixed-length set of complex low pass filter taps.
///
/// Returns the same configuration errors as [`low_pass`].
pub fn low_pass_complex(
    samp_rate: Float,
    cutoff: Float,
    twidth: Float,
    window_type: impl Borrow<WindowType>,
) -> Result<Box<[Complex]>> {
    Ok(low_pass(samp_rate, cutoff, twidth, window_type)?
        .into_iter()
        .map(|t| Complex::new(t, 0.0))
        .collect())
}

fn compute_ntaps(samp_rate: Float, twidth: Float, window_type: &WindowType) -> Result<usize> {
    let a = f64::from(window_type.max_attenuation());
    let count = a * f64::from(samp_rate) / (22.0 * f64::from(twidth));
    // A slice's allocation must fit in isize::MAX bytes. Check before casting,
    // since floating-point to integer casts saturate instead of reporting overflow.
    let max_taps = (isize::MAX as usize) / std::mem::size_of::<Float>();
    if !count.is_finite() || count < 0.0 || count >= max_taps as f64 {
        return Err(Error::msg("Low pass tap count overflow"));
    }
    let t = count as usize;
    Ok(if (t & 1) == 0 { t + 1 } else { t })
}

/// Create a fixed-length set of low pass filter taps.
///
/// Real coefficients can filter either real or complex samples.
/// Returns an error unless the sample rate, cutoff, and transition width are
/// finite and positive, and the cutoff is at most Nyquist. Also rejects tap
/// count overflow, nonfinite window weights, and invalid normalization gain.
pub fn low_pass(
    samp_rate: Float,
    cutoff: Float,
    twidth: Float,
    window_type: impl Borrow<WindowType>,
) -> Result<Box<[Float]>> {
    if !samp_rate.is_finite() || samp_rate <= 0.0 {
        return Err(Error::msg(
            "Low pass sample rate must be finite and positive",
        ));
    }
    if !cutoff.is_finite() || cutoff <= 0.0 || cutoff > samp_rate / 2.0 {
        return Err(Error::msg(
            "Low pass cutoff must be finite, positive, and at most Nyquist",
        ));
    }
    if !twidth.is_finite() || twidth <= 0.0 {
        return Err(Error::msg(
            "Low pass transition width must be finite and positive",
        ));
    }
    let window_type = window_type.borrow();

    let pi = std::f64::consts::PI as Float;
    let ntaps = compute_ntaps(samp_rate, twidth, window_type)?;
    let window = window_type.make_window(ntaps);
    validate_window(&window)?;
    let m = (ntaps - 1) / 2;
    let fwt0 = 2.0 * pi * (cutoff / samp_rate);
    let mut taps: Box<[_]> = window
        .0
        .iter()
        .enumerate()
        .map(|(nm, win)| {
            let n = nm as i64 - m as i64;
            let nf = n as Float;
            if n == 0 {
                fwt0 / pi * win
            } else {
                ((nf * fwt0).sin() / (nf * pi)) * win
            }
        })
        .collect();
    let gain = {
        let mut fmax = taps[m];
        for n in 1..=m {
            fmax += 2.0 * taps[n + m];
        }
        normalization_gain(fmax)?
    };
    for tap in &mut taps {
        *tap *= gain;
    }
    Ok(taps)
}

/// Generate hilbert transformer filter.
///
/// Returns an error unless the window length is odd and greater than one,
/// its weights are finite, and the normalization gain is finite and nonzero.
pub fn hilbert(window: &Window) -> Result<Box<[Float]>> {
    if window.0.len() <= 1 || window.0.len() & 1 == 0 {
        return Err(Error::msg(
            "Hilbert tap count must be odd and greater than one",
        ));
    }
    validate_window(window)?;
    let ntaps = window.0.len();
    let mid = (ntaps - 1) / 2;
    let mut gain = 0.0;
    let mut taps = vec![0.0; ntaps].into_boxed_slice();
    for i in 1..=mid {
        if i & 1 == 1 {
            let x = 1.0 / (i as Float);
            taps[mid + i] = x * window.0[mid + i];
            taps[mid - i] = -x * window.0[mid - i];
            gain = taps[mid + i] - gain;
        } else {
            taps[mid + i] = 0.0;
            taps[mid - i] = 0.0;
        }
    }
    let gain = normalization_gain(2.0 * gain.abs())?;
    for tap in &mut taps {
        *tap *= gain;
    }
    Ok(taps)
}

fn validate_window(window: &Window) -> Result<()> {
    if window.0.iter().any(|weight| !weight.is_finite()) {
        return Err(Error::msg("Filter window weights must be finite"));
    }
    Ok(())
}

fn normalization_gain(gain: Float) -> Result<Float> {
    if !gain.is_finite() || gain == 0.0 || !gain.recip().is_finite() {
        return Err(Error::msg(
            "Filter normalization gain must be finite and nonzero",
        ));
    }
    Ok(gain.recip())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    #[test]
    fn blocked_output_does_not_apply_or_advance_translation() -> Result<()> {
        let (writer, input) = crate::stream::new_stream()?;
        let (mut filter, output) = FirFilter::new(input, [1.0f32])?;
        let control = filter.control();
        control.set_translation(8.0, 2.0)?;
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[Complex::new(1.0, 0.0); 3]);
        window.produce(3, &[]);
        filter.work()?;
        output.read_buf()?.0.consume(3);

        let mut full = filter.dst.write_buf()?;
        let capacity = full.len();
        full.slice().fill(Complex::new(99.0, 0.0));
        full.produce(capacity, &[]);
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&[Complex::new(1.0, 0.0); 2]);
        window.produce(2, &[]);
        control.disable_translation()?;
        assert!(matches!(filter.work()?, BlockRet::WaitForStream(_, 1)));
        // Supersede the blocked disable request: zero frequency must hold the
        // phase reached after the initial three samples (positive pi/2).
        control.set_translation(8.0, 0.0)?;
        output.read_buf()?.0.consume(capacity);
        assert!(matches!(filter.work()?, BlockRet::Again));
        let (window, _) = output.read_buf()?;
        assert_eq!(window.len(), 2);
        for sample in window.iter() {
            assert!((*sample - Complex::new(0.0, 1.0)).norm() < 1e-6);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_configuration_before_processing() {
        let input = || ReadStream::from_slice(&[Complex::default()]);
        assert!(Fir::<Float>::new([]).is_err());
        assert!(FirFilter::<Complex>::new(input(), []).is_err());
        assert!(
            FirFilter::builder([Complex::default()])
                .deci(0)
                .build(input())
                .is_err()
        );
        assert!(
            FirFilter::builder([Complex::default(); 2])
                .deci(usize::MAX)
                .build(input())
                .is_err()
        );
        let capacity = input().total_size();
        assert!(
            FirFilter::builder([Complex::default(); 2])
                .deci(capacity)
                .build(input())
                .is_err()
        );
        for rate in [0.0, -1.0, Float::NAN, Float::INFINITY] {
            assert!(
                FirFilter::builder([Complex::default()])
                    .translate(rate, 0.0)
                    .build(input())
                    .is_err()
            );
        }
        for freq in [Float::NAN, Float::INFINITY] {
            assert!(
                FirFilter::builder([Complex::default()])
                    .translate(48000.0, freq)
                    .build(input())
                    .is_err()
            );
        }
    }

    #[test]
    fn owned_taps_keep_their_allocation() {
        let taps: Box<[Float]> = vec![1.0, 2.0, 3.0].into_boxed_slice();
        let allocation = taps.as_ptr();
        let (block, _) = FirFilter::builder(taps)
            .build(ReadStream::from_slice(&[1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(block.fir.taps.as_ptr(), allocation);
        assert_eq!(&*block.fir.taps, &[3.0, 2.0, 1.0]);
        assert_eq!(block.fir.filter(&[1.0, 0.0, 0.0]), 3.0);
    }

    use super::*;
    use crate::Repeat;
    use crate::blocks::VectorSource;
    use crate::stream::{Tag, TagValue};
    use crate::tests::assert_almost_equal_complex;

    #[test]
    fn test_identity() -> Result<()> {
        let input = vec![
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.2),
            Complex::new(4.1, 0.0),
            Complex::new(5.0, 0.0),
            Complex::new(6.0, 0.2),
        ];
        let taps = vec![Complex::new(1.0, 0.0)];
        for deci in 1..=(3 * input.len()) {
            let (mut src, src_out) = VectorSource::builder(input.clone())
                .repeat(Repeat::finite(2))
                .build()?;
            assert!(matches![src.work()?, BlockRet::Again]);
            assert!(matches![src.work()?, BlockRet::EOF]);

            eprintln!("Testing identity with decimation {deci}");
            let (mut b, os) = FirFilter::builder(taps.clone()).deci(deci).build(src_out)?;
            if deci <= 2 * input.len() {
                assert!(matches![b.work()?, BlockRet::Again]);
            }
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
            let (res, tags) = os.read_buf()?;
            let max = 2 * input.len() / deci;
            if !res.is_empty() {
                assert_eq!(
                    &tags,
                    &[
                        Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                        Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                        Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                        Tag::new(6 / deci, "VectorSource::start", TagValue::Bool(true)),
                        Tag::new(6 / deci, "VectorSource::repeat", TagValue::U64(1)),
                    ]
                );
            }
            assert_almost_equal_complex(
                res.slice(),
                &input
                    .iter()
                    .chain(input.iter())
                    .copied()
                    .step_by(deci)
                    .take(max)
                    .collect::<Vec<_>>(),
            );
        }
        Ok(())
    }

    // Compare frequency translation against manual mixing and filtering.
    #[test]
    fn translate_matches_mixed_input() -> Result<()> {
        let input: Vec<_> = (0..32)
            .map(|i| Complex::new(i as Float, (i as Float) * 0.25))
            .collect();
        let taps = vec![
            Complex::new(0.5, -0.1),
            Complex::new(1.0, 0.2),
            Complex::new(-0.25, 0.05),
            Complex::new(0.125, -0.3),
        ];
        let samp_rate = 8.0;
        let freq = 2.0;
        let deci = 3;
        let phase_step = -2.0 * std::f64::consts::PI * f64::from(freq) / f64::from(samp_rate);
        let rot = Complex::new(phase_step.cos() as Float, phase_step.sin() as Float);
        let mut phase = Complex::new(1.0, 0.0);
        let mixed_input = input
            .iter()
            .map(|&sample| {
                let out = sample * phase;
                phase *= rot;
                out
            })
            .collect::<Vec<_>>();

        let (mut src_a, src_a_out) = VectorSource::new(input.clone())?;
        assert!(matches![src_a.work()?, BlockRet::EOF]);
        let (mut translated, translated_out) = FirFilter::builder(taps.clone())
            .deci(deci)
            .translate(samp_rate, freq)
            .build(src_a_out)?;
        assert!(matches![translated.work()?, BlockRet::Again]);
        assert!(matches![translated.work()?, BlockRet::WaitForStream(_, _)]);

        let (mut src_b, src_b_out) = VectorSource::new(mixed_input)?;
        assert!(matches![src_b.work()?, BlockRet::EOF]);
        let (mut manual, manual_out) = FirFilter::builder(taps).deci(deci).build(src_b_out)?;
        assert!(matches![manual.work()?, BlockRet::Again]);
        assert!(matches![manual.work()?, BlockRet::WaitForStream(_, _)]);

        let (translated_res, _) = translated_out.read_buf()?;
        let (manual_res, _) = manual_out.read_buf()?;
        assert_almost_equal_complex(translated_res.slice(), manual_res.slice());
        Ok(())
    }

    fn tone(len: usize, samp_rate: Float, freq: Float) -> Vec<Complex> {
        let step = 2.0 * std::f64::consts::PI * f64::from(freq) / f64::from(samp_rate);
        (0..len)
            .map(|i| {
                let phase = step * i as f64;
                Complex::new(phase.cos() as Float, phase.sin() as Float)
            })
            .collect()
    }

    #[test]
    fn translated_offset_tone_passes_low_pass() -> Result<()> {
        let samp_rate = 1024.0;
        let freq = 60.0;
        let taps = low_pass_complex(samp_rate, 20.0, 10.0, &WindowType::Hamming).unwrap();
        let input = tone(4096, samp_rate, freq);

        let (mut src, src_out) = VectorSource::new(input)?;
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut filter, out) = FirFilter::builder(taps)
            .translate(samp_rate, freq)
            .build(src_out)?;
        assert!(matches![filter.work()?, BlockRet::Again]);
        assert!(matches![filter.work()?, BlockRet::WaitForStream(_, _)]);

        let (res, _) = out.read_buf()?;
        let mean = res.iter().map(|sample| sample.norm()).sum::<Float>() / res.len() as Float;
        assert!(mean > 0.95, "translated tone mean magnitude: {mean}");
        Ok(())
    }

    #[test]
    fn translated_dc_is_rejected_by_low_pass() -> Result<()> {
        let samp_rate = 1024.0;
        let freq = 60.0;
        let taps = low_pass_complex(samp_rate, 20.0, 10.0, &WindowType::Hamming).unwrap();
        let input = vec![Complex::new(1.0, 0.0); 4096];

        let (mut src, src_out) = VectorSource::new(input)?;
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut filter, out) = FirFilter::builder(taps)
            .translate(samp_rate, freq)
            .build(src_out)?;
        assert!(matches![filter.work()?, BlockRet::Again]);
        assert!(matches![filter.work()?, BlockRet::WaitForStream(_, _)]);

        let (res, _) = out.read_buf()?;
        let mean = res.iter().map(|sample| sample.norm()).sum::<Float>() / res.len() as Float;
        assert!(mean < 0.01, "translated DC mean magnitude: {mean}");
        Ok(())
    }

    #[test]
    fn test_invert() -> Result<()> {
        let input = vec![
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.2),
            Complex::new(4.1, 0.0),
            Complex::new(5.0, 0.0),
            Complex::new(6.0, 0.2),
        ];
        let taps = vec![Complex::new(-1.0, 0.0)];
        for deci in 1..=(input.len() + 1) {
            let (mut src, src_out) = VectorSource::new(input.clone())?;
            src.work()?;

            eprintln!("Testing identity with decimation {deci}");
            let (mut b, os) = FirFilter::builder(taps.clone()).deci(deci).build(src_out)?;
            if deci <= input.len() {
                assert!(matches![b.work()?, BlockRet::Again]);
            }
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
            let (res, _) = os.read_buf()?;
            let max = input.len() / deci;
            assert_almost_equal_complex(
                res.slice(),
                &input
                    .iter()
                    .copied()
                    .step_by(deci)
                    .take(max)
                    .map(|v| -v)
                    .collect::<Vec<_>>(),
            );
        }
        Ok(())
    }

    #[test]
    fn moving_avg() -> Result<()> {
        let input = vec![
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.2),
            Complex::new(4.1, 0.0),
            Complex::new(5.0, 0.0),
            Complex::new(6.0, 0.2),
        ];
        let taps = vec![Complex::new(0.5, 0.0), Complex::new(0.5, 0.0)];
        for deci in 1..=(input.len() + 1) {
            let (mut src, src_out) = VectorSource::new(input.clone())?;
            src.work()?;

            eprintln!("Testing identity with decimation {deci}");
            let (mut b, os) = FirFilter::builder(taps.clone()).deci(deci).build(src_out)?;
            if deci < input.len() {
                assert!(matches![b.work()?, BlockRet::Again]);
            }
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, _)]);
            let (res, _) = os.read_buf()?;
            let max = (input.len() - 1) / deci;
            assert_almost_equal_complex(
                res.slice(),
                &[
                    Complex::new(1.5, 0.0),
                    Complex::new(2.5, 0.1),
                    Complex::new(3.55, 0.1),
                    Complex::new(4.55, 0.0),
                    Complex::new(5.5, 0.1),
                ]
                .into_iter()
                .step_by(deci)
                .take(max)
                .collect::<Vec<_>>(),
            );
        }
        Ok(())
    }

    #[test]
    fn test_complex() {
        let input = vec![
            Complex::new(1.0, 0.0),
            Complex::new(2.0, 0.0),
            Complex::new(3.0, 0.2),
            Complex::new(4.1, 0.0),
            Complex::new(5.0, 0.0),
            Complex::new(6.0, 0.2),
        ];
        let taps = vec![
            Complex::new(0.1, 0.0),
            Complex::new(1.0, 0.0),
            Complex::new(0.0, 0.2),
        ];
        let filter = Fir::new(taps).unwrap();
        assert_almost_equal_complex(
            &filter.filter_n(&input, 1),
            &[
                Complex::new(2.3, 0.22),
                Complex::new(3.41, 0.6),
                Complex::new(4.56, 0.6),
                Complex::new(5.6, 0.84),
            ],
        );
        assert_almost_equal_complex(
            &filter.filter_n(&input, 2),
            &[Complex::new(2.3, 0.22), Complex::new(4.56, 0.6)],
        );
    }

    #[test]
    fn test_filter_generator() {
        let taps = low_pass_complex(10000.0, 1000.0, 1000.0, WindowType::Hamming).unwrap();
        assert_eq!(taps.len(), 25);
        assert_almost_equal_complex(
            &taps,
            &[
                Complex::new(0.002010403, 0.0),
                Complex::new(0.0016210203, 0.0),
                Complex::new(7.851862e-10, 0.0),
                Complex::new(-0.0044467063, 0.0),
                Complex::new(-0.011685465, 0.0),
                Complex::new(-0.018134259, 0.0),
                Complex::new(-0.016773716, 0.0),
                Complex::new(-3.6538055e-9, 0.0),
                Complex::new(0.0358771, 0.0),
                Complex::new(0.08697697, 0.0),
                Complex::new(0.14148787, 0.0),
                Complex::new(0.18345332, 0.0),
                Complex::new(0.19922684, 0.0),
                Complex::new(0.1834533, 0.0),
                Complex::new(0.14148785, 0.0),
                Complex::new(0.08697697, 0.0),
                Complex::new(0.035877097, 0.0),
                Complex::new(-3.6538053e-9, 0.0),
                Complex::new(-0.016773716, 0.0),
                Complex::new(-0.018134257, 0.0),
                Complex::new(-0.011685458, 0.0),
                Complex::new(-0.0044467044, 0.0),
                Complex::new(7.851859e-10, 0.0),
                Complex::new(0.0016210207, 0.0),
                Complex::new(0.002010403, 0.0),
            ],
        );
    }

    #[test]
    fn multiband_full_band_is_centered_impulse() {
        let taps = multiband(&[(0.0, 1.0)], 8, &Window(Box::new([1.0; 8]))).unwrap();
        let mut expected = [Complex::default(); 8];
        expected[4] = Complex::new(8.0_f32.sqrt(), 0.0);
        assert_almost_equal_complex(&taps, &expected);
    }

    #[test]
    fn multiband_rejects_invalid_ranges() {
        assert!(multiband(&[(0.0, 1.0)], 0, &Window(Box::default())).is_err());
        assert!(multiband(&[(0.0, 3.0)], 8, &Window(Box::new([1.0; 8]))).is_err());
        assert!(multiband(&[(0.5, 0.25)], 8, &Window(Box::new([1.0; 8]))).is_err());
        assert!(multiband(&[(0.0, 1.0)], 7, &Window(Box::new([1.0; 8]))).is_err());
        for band in [(-0.1, 0.5), (Float::NAN, 0.5), (0.0, Float::INFINITY)] {
            assert!(multiband(&[band], 8, &Window(Box::new([1.0; 8]))).is_err());
        }
        assert!(multiband(&[(0.0, 1.0)], 3, &Window(Box::new([Float::NAN; 3]))).is_err());
    }

    #[test]
    fn low_pass_rejects_invalid_design_parameters() {
        let window = WindowType::Hamming;
        for invalid in [0.0, -1.0, Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            assert!(low_pass(invalid, 100.0, 100.0, &window).is_err());
            assert!(low_pass(1000.0, invalid, 100.0, &window).is_err());
            assert!(low_pass(1000.0, 100.0, invalid, &window).is_err());
            assert!(low_pass_complex(1000.0, invalid, 100.0, &window).is_err());
        }
        assert!(low_pass(1000.0, 501.0, 100.0, &window).is_err());
        assert!(low_pass(1000.0, 100.0, Float::MIN_POSITIVE, &window).is_err());
        assert!(low_pass(1000.0, 100.0, 100.0, WindowType::HammingParm(Float::NAN)).is_err());
    }

    #[test]
    fn low_pass_accepts_nyquist_and_large_finite_rates() -> Result<()> {
        for rate in [1000.0, Float::MAX] {
            let taps = low_pass(rate, rate / 2.0, rate / 10.0, WindowType::Hamming)?;
            assert!(taps.iter().all(|tap| tap.is_finite()));
            assert!((taps.iter().sum::<Float>() - 1.0).abs() < 0.0001);
        }
        Ok(())
    }

    #[test]
    fn hilbert_rejects_invalid_windows() {
        for weights in [
            vec![],
            vec![1.0],
            vec![1.0; 2],
            vec![1.0; 4],
            vec![0.0; 3],
            vec![Float::NAN; 3],
            vec![Float::INFINITY; 3],
        ] {
            assert!(hilbert(&Window(weights.into_boxed_slice())).is_err());
        }
    }

    #[test]
    fn hilbert_generates_normalized_antisymmetric_taps() -> Result<()> {
        let taps = hilbert(&Window(Box::new([1.0; 3])))?;
        assert_eq!(&*taps, &[-0.5, 0.0, 0.5]);
        Ok(())
    }
}
