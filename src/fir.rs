//! Finite impulse response filter.
//!
//! If using many taps, [`FftFilter`](crate::blocks::FftFilter) probably has
//! better performance.
//!
//! TODO: Change taps to return error instead of assert?
/*
 * TODO:
 * * Only handles case where input, output, and tap type are all the same.
 */
use log::error;
use std::borrow::Borrow;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::window::{Window, WindowType};
use crate::{Complex, Float, Result, Sample};

#[doc(hidden)]
pub trait FrequencyTranslate {
    /// Per-block translation state.
    ///
    /// Most sample types do not support translation and use `()`. Complex FIRs
    /// use a rotator that advances once per output sample.
    type Translator: Send;

    /// Return the no-op translator for blocks that do not translate frequency.
    fn no_translation() -> Self::Translator;

    /// Configure frequency translation before the FIR is built.
    ///
    /// Implementations may modify `taps` to fold fixed per-tap phase terms into
    /// the filter, then return any state needed to finish translation while
    /// samples are produced.
    fn new_translator(
        taps: &mut [Self],
        samp_rate: Float,
        freq: Float,
        deci: usize,
    ) -> Self::Translator
    where
        Self: Sized;

    /// Apply the continuing part of frequency translation to produced samples.
    fn translate_output(out: &mut [Self], translator: &mut Self::Translator)
    where
        Self: Sized;
}

/// Finite impulse response filter.
pub struct Fir<T> {
    taps: Vec<T>,
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
        assert_eq!(vec1.len(), vec2.len());
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
            .fold(partial, |acc, (&f, &x)| acc + x * f)
    }
}

impl Fir<Float> {
    /// Run filter once, creating one sample from the taps and an
    /// equal number of input samples.
    #[must_use]
    pub fn filter_float(&self, input: &[Float]) -> Float {
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
                .fold(partial, |acc, (&f, &x)| acc + x * f);
        }
        #[allow(unreachable_code)]
        self.filter(input)
    }
}

impl<T> Fir<T>
where
    T: Sample + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    /// Create new FIR.
    #[must_use]
    pub fn new(taps: impl AsRef<[T]>) -> Self {
        let taps = taps.as_ref();
        assert!(!taps.is_empty());
        Self {
            taps: taps.iter().copied().rev().collect(),
        }
    }
    /// Run filter once, creating one sample from the taps and an
    /// equal number of input samples.
    #[must_use]
    pub fn filter(&self, input: &[T]) -> T {
        assert!(
            input.len() >= self.taps.len(),
            "input {} < taps {}",
            input.len(),
            self.taps.len()
        );
        input
            .iter()
            .zip(self.taps.iter())
            .fold(T::default(), |acc, (&f, &x)| acc + x * f)
    }

    /// Call `filter()` multiple times, across an input range.
    #[must_use]
    pub fn filter_n(&self, input: &[T], deci: usize) -> Vec<T> {
        let n = input.len() - self.taps.len();
        (0..=n)
            .step_by(deci)
            .map(|i| self.filter(&input[i..]))
            .collect()
    }

    /// Like `filter_n`, but avoids a copy when there's a destination in mind.
    pub fn filter_n_inplace(&self, input: &[T], deci: usize, out: &mut [T]) {
        out.iter_mut()
            .enumerate()
            .for_each(|(i, o)| *o = self.filter(&input[(i * deci)..]));
    }
}

/// Builder for a FIR filter block.
///
/// A builder is needed to create a decimating FIR filter block.
pub struct FirFilterBuilder<T> {
    taps: Vec<T>,
    deci: usize,
    // Optional `(sample_rate, frequency)` requested by `translate()`.
    translate: Option<(Float, Float)>,
}

impl<T> FirFilterBuilder<T>
where
    T: Sample + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T> + FrequencyTranslate,
{
    /// Set the decimation to the given value.
    ///
    /// The default is 1, meaning no decimation.
    #[must_use]
    pub fn deci(mut self, deci: usize) -> Self {
        assert_ne!(deci, 0);
        self.deci = deci;
        self
    }

    /// Build a `FirFilter` with the provided settings.
    #[must_use]
    pub fn build(self, src: ReadStream<T>) -> (FirFilter<T>, ReadStream<T>) {
        let FirFilterBuilder {
            mut taps,
            deci,
            translate,
        } = self;
        let translator = translate.map_or_else(T::no_translation, |(samp_rate, freq)| {
            T::new_translator(&mut taps, samp_rate, freq, deci)
        });
        let (mut block, stream) = FirFilter::new(src, &taps);
        block.deci = deci;
        block.translator = translator;
        (block, stream)
    }
}

/// Finite impulse response filter block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FirFilter<T: Sample + FrequencyTranslate> {
    fir: Fir<T>,
    ntaps: usize,
    deci: usize,
    // Per-block rotator state for fused frequency translation.
    translator: T::Translator,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> FirFilter<T>
where
    T: Sample + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T> + FrequencyTranslate,
{
    /// Create new `FirFilterBuilder`, with the supplied taps.
    pub fn builder(taps: impl Into<Vec<T>>) -> FirFilterBuilder<T> {
        FirFilterBuilder {
            taps: taps.into(),
            deci: 1,
            translate: None,
        }
    }
    /// Create Fir block given taps.
    pub fn new(src: ReadStream<T>, taps: impl AsRef<[T]>) -> (Self, ReadStream<T>) {
        let taps = taps.as_ref();
        assert!(!taps.is_empty());
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                ntaps: taps.len(),
                deci: 1,
                translator: T::no_translation(),
                fir: Fir::new(taps),
            },
            dr,
        )
    }
}

macro_rules! impl_no_frequency_translate {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FrequencyTranslate for $ty {
                type Translator = ();
                fn no_translation() -> Self::Translator {}

                fn new_translator(
                    _taps: &mut [Self],
                    _samp_rate: Float,
                    _freq: Float,
                    _deci: usize,
                ) -> Self::Translator {
                    error!("FirFilter asked to translate on non-Complex");
                }

                fn translate_output(_out: &mut [Self], _translator: &mut Self::Translator) {}
            }
        )*
    };
}

// Enable FftFilter on these types, even though only `Complex` actually supports
// frequency translation. This is type restricted in the builder, so it should
// not be possible to actually instantiate a translator anyway.
impl_no_frequency_translate!(f32, f64, u8, u32, i32);

#[doc(hidden)]
pub struct ComplexFrequencyTranslator {
    /// Current complex oscillator value for the next output sample.
    phase: Complex,
    /// Per-output oscillator step, including decimation.
    step: Complex,
}

impl FrequencyTranslate for Complex {
    type Translator = Option<ComplexFrequencyTranslator>;

    fn no_translation() -> Self::Translator {
        None
    }

    fn new_translator(
        taps: &mut [Self],
        samp_rate: Float,
        freq: Float,
        deci: usize,
    ) -> Self::Translator {
        assert_ne!(samp_rate, 0.0);
        assert_ne!(deci, 0);
        if freq == 0.0 {
            return None;
        }
        let input_step = 2.0 * std::f64::consts::PI * f64::from(freq) / f64::from(samp_rate);
        let tap_step = Complex::new(input_step.cos() as Float, input_step.sin() as Float);
        let mut phase = Complex::new(1.0, 0.0);
        // Pre-rotate taps by their relative delay so filtering plus one output
        // rotation matches explicitly mixing each input sample by `-freq`.
        for tap in &mut *taps {
            *tap *= phase;
            phase *= tap_step;
        }

        // The first produced FIR output is aligned with the newest sample in
        // the first input window; after that, outputs advance by `deci` inputs.
        let first_output_phase = -input_step * (taps.len() - 1) as f64;
        let output_step = -input_step * deci as f64;
        Some(ComplexFrequencyTranslator {
            phase: Complex::new(
                first_output_phase.cos() as Float,
                first_output_phase.sin() as Float,
            ),
            step: Complex::new(output_step.cos() as Float, output_step.sin() as Float),
        })
    }

    fn translate_output(out: &mut [Self], translator: &mut Self::Translator) {
        // TODO: do we need to reset this periodically, to get around rounding
        // errors?
        if let Some(translator) = translator {
            for sample in out {
                *sample *= translator.phase;
                translator.phase *= translator.step;
            }
        }
    }
}

impl FirFilterBuilder<Complex> {
    /// Mix by `-freq` Hz while filtering.
    ///
    /// A tone at `freq` Hz is translated to DC. The implementation folds the
    /// input mixer into the FIR taps and keeps one rotator per output sample.
    #[must_use]
    pub fn translate(mut self, samp_rate: Float, freq: Float) -> Self {
        self.translate = Some((samp_rate, freq));
        self
    }
}

impl<T> Block for FirFilter<T>
where
    T: Sample + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T> + FrequencyTranslate,
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
        let n = std::cmp::min(n, out.len() * self.deci);

        // Final `n` (samples to consume) calculated. Sanity check it.
        assert_eq!(n % self.deci, 0);
        assert_ne!(n, 0, "input: {} out: {}", input.len(), out.len());

        // Run the FIR.
        let out_n = n / self.deci;
        self.fir
            .filter_n_inplace(&input.slice()[..need], self.deci, &mut out.slice()[..out_n]);

        // Frequency translate. This is an empty function call if translation is
        // zero.
        T::translate_output(&mut out.slice()[..out_n], &mut self.translator);

        // Sanity check the generated output.
        assert!(out_n <= out.len());

        tags.retain(|tag| tag.pos() < n);
        input.consume(n);
        if self.deci == 1 {
            out.produce(out_n, &tags);
        } else {
            for t in &mut tags {
                t.set_pos(t.pos() / self.deci);
            }
            out.produce(out_n, &tags);
        }
        // While we could keep track of which stream is the constraining factor,
        // the code is simpler if work() is just called again, and the right
        // WaitForStream is returned above instead.
        Ok(BlockRet::Again)
    }
}

/// Create a multiband filter.
///
/// TODO: this is untested.
#[must_use]
pub fn multiband(bands: &[(Float, Float)], taps: usize, window: &Window) -> Option<Vec<Complex>> {
    use rustfft::FftPlanner;

    if taps == 0 || taps != window.0.len() {
        return None;
    }

    let mut ideal = vec![Complex::new(0.0, 0.0); taps];
    let scale = (taps as Float) / 2.0;
    for (low, high) in bands {
        let a = (low * scale).floor() as usize;
        let b = (high * scale).ceil() as usize;
        if a > taps || b > taps {
            return None;
        }
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
    Some(
        ideal
            .into_iter()
            .enumerate()
            .map(|(n, v)| v * window.0[n] / Complex::new(scale, 0.0))
            .collect(),
    )
}

/// Create taps for a low pass filter as complex taps.
#[must_use]
pub fn low_pass_complex(
    samp_rate: Float,
    cutoff: Float,
    twidth: Float,
    window_type: impl Borrow<WindowType>,
) -> Vec<Complex> {
    low_pass(samp_rate, cutoff, twidth, window_type)
        .into_iter()
        .map(|t| Complex::new(t, 0.0))
        .collect()
}

fn compute_ntaps(samp_rate: Float, twidth: Float, window_type: &WindowType) -> usize {
    let a = window_type.max_attenuation();
    let t = (a * samp_rate / (22.0 * twidth)) as usize;
    if (t & 1) == 0 { t + 1 } else { t }
}

/// Create taps for a low pass filter.
///
/// TODO: this could be faster if we supported filtering a Complex by a Float.
/// A low pass filter doesn't actually need complex taps.
#[must_use]
pub fn low_pass(
    samp_rate: Float,
    cutoff: Float,
    twidth: Float,
    window_type: impl Borrow<WindowType>,
) -> Vec<Float> {
    let window_type = window_type.borrow();

    let pi = std::f64::consts::PI as Float;
    let ntaps = compute_ntaps(samp_rate, twidth, window_type);
    let window = window_type.make_window(ntaps);
    let m = (ntaps - 1) / 2;
    let fwt0 = 2.0 * pi * cutoff / samp_rate;
    let taps: Vec<_> = window
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
        let gain: Float = 1.0;
        let mut fmax = taps[m];
        for n in 1..=m {
            fmax += 2.0 * taps[n + m];
        }
        gain / fmax
    };
    taps.into_iter().map(|t| t * gain).collect()
}

/// Generate hilbert transformer filter.
#[must_use]
pub fn hilbert(window: &Window) -> Vec<Float> {
    assert!(!window.0.is_empty());
    assert_ne!(window.0.len(), 1);
    let ntaps = window.0.len();
    let mid = (ntaps - 1) / 2;
    let mut gain = 0.0;
    let mut taps = vec![0.0; ntaps];
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
    let gain = 1.0 / (2.0 * gain.abs());
    taps.iter().map(|e| gain * *e).collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
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
            let (mut b, os) = FirFilter::builder(taps.clone()).deci(deci).build(src_out);
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

        let (mut src_a, src_a_out) = VectorSource::new(input.clone());
        assert!(matches![src_a.work()?, BlockRet::EOF]);
        let (mut translated, translated_out) = FirFilter::builder(taps.clone())
            .deci(deci)
            .translate(samp_rate, freq)
            .build(src_a_out);
        assert!(matches![translated.work()?, BlockRet::Again]);
        assert!(matches![translated.work()?, BlockRet::WaitForStream(_, _)]);

        let (mut src_b, src_b_out) = VectorSource::new(mixed_input);
        assert!(matches![src_b.work()?, BlockRet::EOF]);
        let (mut manual, manual_out) = FirFilter::builder(taps).deci(deci).build(src_b_out);
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
        let taps = low_pass_complex(samp_rate, 20.0, 10.0, &WindowType::Hamming);
        let input = tone(4096, samp_rate, freq);

        let (mut src, src_out) = VectorSource::new(input);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut filter, out) = FirFilter::builder(taps)
            .translate(samp_rate, freq)
            .build(src_out);
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
        let taps = low_pass_complex(samp_rate, 20.0, 10.0, &WindowType::Hamming);
        let input = vec![Complex::new(1.0, 0.0); 4096];

        let (mut src, src_out) = VectorSource::new(input);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut filter, out) = FirFilter::builder(taps)
            .translate(samp_rate, freq)
            .build(src_out);
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
            let (mut src, src_out) = VectorSource::new(input.clone());
            src.work()?;

            eprintln!("Testing identity with decimation {deci}");
            let (mut b, os) = FirFilter::builder(taps.clone()).deci(deci).build(src_out);
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
            let (mut src, src_out) = VectorSource::new(input.clone());
            src.work()?;

            eprintln!("Testing identity with decimation {deci}");
            let (mut b, os) = FirFilter::builder(taps.clone()).deci(deci).build(src_out);
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
        let filter = Fir::new(&taps);
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
        let taps = low_pass_complex(10000.0, 1000.0, 1000.0, WindowType::Hamming);
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
    fn multiband_rejects_invalid_ranges() {
        assert!(multiband(&[(0.0, 1.0)], 0, &Window(vec![])).is_none());
        assert!(multiband(&[(0.0, 3.0)], 8, &Window(vec![1.0; 8])).is_none());
    }
}
