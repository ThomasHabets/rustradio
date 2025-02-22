/*! Finite impulse response filter.

Use FftFilter if many taps are used, for better performance.
*/
/*
 * TODO:
 * * Only handles case where input, output, and tap type are all the same.
 */
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::window::{Window, WindowType};
use crate::{Complex, Error, Float};

/// Finite impulse response filter.
pub struct FIR<T: Copy> {
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

impl FIR<Float> {
    /// Run filter once, creating one sample from the taps and an
    /// equal number of input samples.
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
            let batch_n = 8;
            // How will this work if Float is f64?
            type Batch = std::simd::f32x8;
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

impl<T> FIR<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    /// Create new FIR.
    pub fn new(taps: &[T]) -> Self {
        Self {
            taps: taps.iter().copied().rev().collect(),
        }
    }
    /// Run filter once, creating one sample from the taps and an
    /// equal number of input samples.
    pub fn filter(&self, input: &[T]) -> T {
        input
            .iter()
            .zip(self.taps.iter())
            .fold(T::default(), |acc, (&f, &x)| acc + x * f)
    }

    /// Call `filter()` multiple times, across an input range.
    pub fn filter_n(&self, input: &[T]) -> Vec<T> {
        let n = input.len() - self.taps.len() + 1;
        (0..n).map(|i| self.filter(&input[i..])).collect()
    }
}

/// Finite impulse response filter block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FIRFilter<T: Copy> {
    fir: FIR<T>,
    ntaps: usize,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Copy> FIRFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    /// Create FIR block given taps.
    pub fn new(src: ReadStream<T>, taps: &[T]) -> (Self, ReadStream<T>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                ntaps: taps.len(),
                fir: FIR::new(taps),
            },
            dr,
        )
    }
}

impl<T> Block for FIRFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        // TODO: is this right, with the "plus one"?
        let need = self.ntaps + 1;
        let (input, tags) = self.src.read_buf()?;
        if input.len() < need {
            return Ok(BlockRet::WaitForStream(&self.src, need));
        }

        let mut out = self.dst.write_buf()?;
        if out.len() < need {
            return Ok(BlockRet::WaitForStream(&self.dst, need));
        }

        let n = std::cmp::min(input.len(), out.len());
        let v = self.fir.filter_n(&input.slice()[..n]);
        assert!(v.len() <= n);
        let n = v.len();
        input.consume(n);
        out.fill_from_iter(v);
        out.produce(n, &tags);
        Ok(BlockRet::Ok)
    }
}

/// Create a multiband filter.
///
/// TODO: this is untested.
pub fn multiband(bands: &[(Float, Float)], taps: usize, window: &Window) -> Option<Vec<Complex>> {
    if taps != window.0.len() {
        return None;
    }
    use rustfft::FftPlanner;

    let mut ideal = vec![Complex::new(0.0, 0.0); taps];
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
    Some(
        ideal
            .into_iter()
            .enumerate()
            .map(|(n, v)| v * window.0[n] / Complex::new(scale, 0.0))
            .collect(),
    )
}

/// Create taps for a low pass filter as complex taps.
pub fn low_pass_complex(
    samp_rate: Float,
    cutoff: Float,
    twidth: Float,
    window_type: &WindowType,
) -> Vec<Complex> {
    low_pass(samp_rate, cutoff, twidth, window_type)
        .into_iter()
        .map(|t| Complex::new(t, 0.0))
        .collect()
}

fn compute_ntaps(samp_rate: Float, twidth: Float, window_type: &WindowType) -> usize {
    let a = window_type.max_attenuation();
    let t = (a * samp_rate / (22.0 * twidth)) as usize;
    if (t & 1) == 0 {
        t + 1
    } else {
        t
    }
}

/// Create taps for a low pass filter.
///
/// TODO: this could be faster if we supported filtering a Complex by a Float.
/// A low pass filter doesn't actually need complex taps.
pub fn low_pass(
    samp_rate: Float,
    cutoff: Float,
    twidth: Float,
    window_type: &WindowType,
) -> Vec<Float> {
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
pub fn hilbert(window: &Window) -> Vec<Float> {
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
mod tests {
    use super::*;
    use crate::tests::assert_almost_equal_complex;

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
        let filter = FIR::new(&taps);
        assert_almost_equal_complex(
            &filter.filter_n(&input),
            &[
                Complex::new(2.3, 0.22),
                Complex::new(3.41, 0.6),
                Complex::new(4.56, 0.6),
                Complex::new(5.6, 0.84),
            ],
        );
    }

    #[test]
    fn test_filter_generator() {
        let taps = low_pass_complex(10000.0, 1000.0, 1000.0, &WindowType::Hamming);
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
}
