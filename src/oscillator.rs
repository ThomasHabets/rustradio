//! Stream-independent complex oscillator shared by sources and mixers.

use crate::{AlgebraicMul, Complex, Error, Float, Result};

pub(crate) const NORMALIZE_INTERVAL: usize = 4096;

/// Infinite iterator of unit complex phasors.
///
/// Sample `n`, starting at zero, is approximately
/// `exp(i * (phase + n * 2π * frequency / sample_rate))`. Configuration and
/// output use [`Float`]. A recursive double-precision phasor with periodic
/// normalization avoids drift. No streams or output buffers are allocated.
///
/// ```
/// use rustradio::{Complex, oscillator::ComplexOscillator};
/// let mut oscillator = ComplexOscillator::new(8.0, 2.0)?;
/// assert_eq!(oscillator.next(), Some(Complex::new(1.0, 0.0)));
/// oscillator.set_frequency(8.0, -2.0)?;
/// # Ok::<(), rustradio::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct ComplexOscillator {
    rotation: num_complex::Complex<f64>,
    phasor: num_complex::Complex<f64>,
    until_normalize: usize,
}

impl ComplexOscillator {
    /// Create an oscillator whose first sample has phase zero.
    ///
    /// Rejects nonfinite configuration and nonpositive sample rates. Negative
    /// frequencies are supported; frequencies alias modulo the sample rate.
    pub fn new(sample_rate: Float, frequency: Float) -> Result<Self> {
        let mut oscillator = Self {
            rotation: num_complex::Complex::new(1.0, 0.0),
            phasor: num_complex::Complex::new(1.0, 0.0),
            until_normalize: NORMALIZE_INTERVAL,
        };
        oscillator.set_frequency(sample_rate, frequency)?;
        Ok(oscillator)
    }

    /// Set frequency in Hz, preserving the phase of the next returned sample.
    ///
    /// Rejects nonfinite values and nonpositive sample rates without changing
    /// the oscillator. Range reduction before division handles extreme ratios.
    pub fn set_frequency(&mut self, sample_rate: Float, frequency: Float) -> Result<()> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 || !frequency.is_finite() {
            return Err(Error::msg(
                "oscillator requires a finite positive sample rate and finite frequency",
            ));
        }
        let frequency = f64::from(frequency);
        let sample_rate = f64::from(sample_rate);
        let step = (frequency % sample_rate) / sample_rate * std::f64::consts::TAU;
        let (sin, cos) = step.sin_cos();
        self.rotation = num_complex::Complex::new(cos, sin);
        Ok(())
    }

    /// Set the phase in radians of the next returned sample.
    ///
    /// Rejects nonfinite values without changing the oscillator.
    pub fn set_phase(&mut self, phase: Float) -> Result<()> {
        if !phase.is_finite() {
            return Err(Error::msg("oscillator phase must be finite"));
        }
        let (sin, cos) = f64::from(phase).sin_cos();
        self.phasor = num_complex::Complex::new(cos, sin);
        self.until_normalize = NORMALIZE_INTERVAL;
        Ok(())
    }

    /// Multiply each input sample by the next oscillator sample, writing output.
    ///
    /// Input and output must be the same length.
    ///
    /// This is equivalent to the following, but allows the compiler to auto
    /// vectorize:
    ///
    /// ```
    /// # let input: Vec<rustradio::Complex> = vec![];
    /// # let mut output: Vec<rustradio::Complex> = vec![];
    /// # let mut oscillator = rustradio::oscillator::ComplexOscillator::new(100.0,10.0).unwrap();
    /// for n in 0..input.len() {
    ///     output[n] = input[n] * oscillator.next().unwrap();
    /// }
    /// ```
    pub fn mix(&mut self, input: &[Complex], output: &mut [Complex]) -> Result<()> {
        const LANES: usize = 8;
        if input.len() != output.len() {
            return Err(Error::msg(format!(
                "oscillator input len ({}) and output len ({}) must match",
                input.len(),
                output.len()
            )));
        }
        // Independent phase lanes let LLVM vectorize consecutive samples.
        // Keep normalization outside the lane loop and use the iterator for tails.
        let mut offset = 0;
        while input.len() - offset >= LANES {
            if self.until_normalize < LANES {
                output[offset] =
                    input[offset].algebraic_mul(self.next().expect("infinite oscillator"));
                offset += 1;
                continue;
            }
            let count = (input.len() - offset).min(self.until_normalize) / LANES * LANES;
            let mut real = [0.0; LANES];
            let mut imag = [0.0; LANES];
            let mut phase = self.phasor;
            let mut stride = num_complex::Complex::new(1.0, 0.0);
            for lane in 0..LANES {
                real[lane] = phase.re;
                imag[lane] = phase.im;
                phase = phase.algebraic_mul(self.rotation);
                stride = stride.algebraic_mul(self.rotation);
            }
            for (src, dst) in input[offset..offset + count]
                .as_chunks::<LANES>()
                .0
                .iter()
                .zip(
                    output[offset..offset + count]
                        .as_chunks_mut::<LANES>()
                        .0
                        .iter_mut(),
                )
            {
                for lane in 0..LANES {
                    dst[lane] = src[lane]
                        .algebraic_mul(Complex::new(real[lane] as Float, imag[lane] as Float));
                    let next =
                        num_complex::Complex::new(real[lane], imag[lane]).algebraic_mul(stride);
                    real[lane] = next.re;
                    imag[lane] = next.im;
                }
            }
            self.phasor = num_complex::Complex::new(real[0], imag[0]);
            self.until_normalize -= count;
            if self.until_normalize == 0 {
                self.phasor = self.phasor.algebraic_mul(1.0 / self.phasor.norm());
                self.until_normalize = NORMALIZE_INTERVAL;
            }
            offset += count;
        }
        for (sample, out) in input[offset..].iter().zip(output[offset..].iter_mut()) {
            *out = sample.algebraic_mul(self.next().expect("infinite oscillator"));
        }
        Ok(())
    }

    /// Phase of the next sample in radians, in the range [-π, π].
    #[must_use]
    pub fn phase(&self) -> Float {
        self.phasor.arg() as Float
    }
}

impl Iterator for ComplexOscillator {
    type Item = Complex;

    #[inline]
    fn next(&mut self) -> Option<Complex> {
        let sample = Complex::new(self.phasor.re as Float, self.phasor.im as Float);
        self.phasor = self.phasor.algebraic_mul(self.rotation);
        self.until_normalize -= 1;
        if self.until_normalize == 0 {
            self.phasor = self.phasor.algebraic_mul(1.0f64 / self.phasor.norm());
            self.until_normalize = NORMALIZE_INTERVAL;
        }
        Some(sample)
    }
}

impl std::iter::FusedIterator for ComplexOscillator {}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(phase: f64) -> Complex {
        Complex::new(phase.cos() as Float, phase.sin() as Float)
    }

    #[test]
    fn first_phase_signed_frequencies_and_aliases() -> Result<()> {
        for frequency in [0.0, 1.0, -1.0, 4.0, -4.0, 17.0] {
            let mut oscillator = ComplexOscillator::new(8.0, frequency)?;
            oscillator.set_phase(0.37)?;
            let step = (f64::from(frequency) % 8.0) / 8.0 * std::f64::consts::TAU;
            for index in 0..2 * NORMALIZE_INTERVAL + 1 {
                let actual = oscillator.next().unwrap();
                assert!(
                    (actual - expected(f64::from(0.37_f32) + step * index as f64)).norm() < 1e-6
                );
            }
        }
        Ok(())
    }

    #[test]
    fn retuning_preserves_next_phase_and_phase_can_be_reset() -> Result<()> {
        let mut oscillator = ComplexOscillator::new(16.0, 1.0)?;
        oscillator.set_phase(0.5)?;
        for _ in 0..3 {
            oscillator.next();
        }
        let mut original = oscillator.clone();
        let phase = oscillator.phase();
        oscillator.set_frequency(16.0, -2.0)?;
        assert_eq!(oscillator.phase(), phase);
        assert_eq!(oscillator.next(), original.next());
        assert!(
            (oscillator.next().unwrap() - expected(f64::from(phase) - std::f64::consts::FRAC_PI_4))
                .norm()
                < 1e-6
        );
        oscillator.set_phase(-1.2)?;
        assert_eq!(oscillator.phase(), -1.2);
        assert_eq!(oscillator.next(), Some(expected(f64::from(-1.2_f32))));
        Ok(())
    }

    #[test]
    fn invalid_settings_leave_iteration_unchanged() -> Result<()> {
        let mut oscillator = ComplexOscillator::new(8.0, 1.0)?;
        oscillator.set_phase(0.2)?;
        let mut original = oscillator.clone();
        for rate in [0.0, -1.0, Float::NAN, Float::INFINITY] {
            assert!(ComplexOscillator::new(rate, 1.0).is_err());
            assert!(oscillator.set_frequency(rate, 1.0).is_err());
        }
        for invalid in [Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            assert!(ComplexOscillator::new(8.0, invalid).is_err());
            assert!(oscillator.set_frequency(8.0, invalid).is_err());
            assert!(oscillator.set_phase(invalid).is_err());
        }
        for _ in 0..10 {
            assert_eq!(oscillator.next(), original.next());
        }
        Ok(())
    }

    #[test]
    fn extreme_frequency_ratios_remain_finite() -> Result<()> {
        for frequency in [Float::MAX, -Float::MAX] {
            let oscillator = ComplexOscillator::new(Float::from_bits(1), frequency)?;
            for sample in oscillator.take(10) {
                assert!(sample.re.is_finite() && sample.im.is_finite());
                assert!((sample.norm() - 1.0).abs() < 1e-6);
            }
        }
        Ok(())
    }

    #[test]
    fn mixing_matches_iteration_across_batches_and_retuning() -> Result<()> {
        let input: Vec<_> = (0..2 * NORMALIZE_INTERVAL + 7)
            .map(|i| Complex::new((i % 13) as Float - 6.0, (i % 7) as Float - 3.0))
            .collect();
        for frequency in [0.0, 100_000.0, -100_000.0] {
            let mut batch = ComplexOscillator::new(1_024_000.0, frequency)?;
            batch.set_phase(0.37)?;
            let mut scalar = batch.clone();
            let mut output = vec![Complex::default(); input.len()];
            let mut offset = 0;
            for size in [0, 1, NORMALIZE_INTERVAL - 2, 4, NORMALIZE_INTERVAL + 4] {
                let end = offset + size;
                batch.mix(&input[offset..end], &mut output[offset..end])?;
                for i in offset..end {
                    assert!(
                        (output[i] - input[i].algebraic_mul(scalar.next().unwrap())).norm() < 2e-6
                    );
                }
                assert!((batch.next().unwrap() - scalar.next().unwrap()).norm() < 2e-7);
                batch.set_frequency(1_024_000.0, -frequency)?;
                scalar.set_frequency(1_024_000.0, -frequency)?;
                offset = end;
            }
            assert_eq!(offset, input.len());
        }
        Ok(())
    }

    #[test]
    fn batch_mixing_stays_stable_across_unaligned_normalization_boundaries() -> Result<()> {
        let sizes = [1, 7, 8, 9, 127, 4093, 8193];
        let input = vec![Complex::new(1.0, 0.0); 8193];
        let mut output = vec![Complex::default(); input.len()];
        for frequency in [0.0, 100_000.0, -100_000.0, 1_124_000.0] {
            let mut oscillator = ComplexOscillator::new(1_024_000.0, frequency)?;
            oscillator.set_phase(0.123)?;
            let step = (f64::from(frequency) % 1_024_000.0) / 1_024_000.0 * std::f64::consts::TAU;
            let mut offset = 0;
            let mut batches = sizes.into_iter().cycle();
            while offset < 1_000_000 {
                let n = batches.next().unwrap().min(1_000_000 - offset);
                oscillator.mix(&input[..n], &mut output[..n])?;
                for (i, sample) in output[..n].iter().enumerate() {
                    let phase = f64::from(0.123_f32) + step * (offset + i) as f64;
                    assert!((sample.norm() - 1.0).abs() < 2e-7);
                    assert!((*sample - expected(phase)).norm() < 1e-6);
                }
                offset += n;
            }
            assert!(
                (oscillator.next().unwrap()
                    - expected(f64::from(0.123_f32) + step * offset as f64))
                .norm()
                    < 1e-6
            );
        }
        Ok(())
    }

    #[test]
    fn mismatched_mixing_lengths_leave_output_and_phase_unchanged() -> Result<()> {
        for (input_len, output_len) in [(0, 1), (1, 0), (1, 2), (2, 1)] {
            let mut oscillator = ComplexOscillator::new(8.0, 1.0)?;
            let mut original = oscillator.clone();
            let input = vec![Complex::new(2.0, 3.0); input_len];
            let mut output = vec![Complex::new(99.0, -99.0); output_len];
            let expected = output.clone();
            assert!(oscillator.mix(&input, &mut output).is_err());
            assert_eq!(output, expected);
            assert_eq!(oscillator.next(), original.next());
        }
        Ok(())
    }

    #[test]
    fn normalization_preserves_amplitude_and_phase_over_long_runs() -> Result<()> {
        let frequency = 100_000.0;
        let rate = 1_024_000.0;
        let mut oscillator = ComplexOscillator::new(rate, frequency)?;
        oscillator.set_phase(0.123)?;
        let step = f64::from(frequency) / f64::from(rate) * std::f64::consts::TAU;
        for index in 0..1_000_000 {
            let sample = oscillator.next().unwrap();
            assert!((sample.norm() - 1.0).abs() < 2e-7);
            if (index + 1) % NORMALIZE_INTERVAL == 0 {
                assert!((oscillator.phasor.norm() - 1.0).abs() < 1e-14);
                assert!(
                    (sample - expected(f64::from(0.123_f32) + step * index as f64)).norm() < 1e-6
                );
            }
        }
        Ok(())
    }
}
