//! Generate a pure signal.
use crate::Result;
use crate::oscillator::ComplexOscillator;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Generate a pure complex sine wave.
///
/// Uses a double-precision recursive oscillator with periodic amplitude
/// normalization. Sample `n`, starting at one, is approximately
/// `amplitude * (sin(n * step) - i * cos(n * step))`, where
/// `step = 2π * freq / samp_rate`. No tags are generated.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SignalSourceComplex {
    #[rustradio(out)]
    dst: WriteStream<Complex>,

    amplitude: Float,
    oscillator: ComplexOscillator,
}

impl SignalSourceComplex {
    /// Create a complex sine source.
    ///
    /// Returns an error unless the sample rate is finite and positive and the
    /// frequency and amplitude are finite. Negative frequencies are supported.
    pub fn new(
        samp_rate: Float,
        freq: Float,
        amplitude: Float,
    ) -> Result<(Self, ReadStream<Complex>)> {
        if !amplitude.is_finite() {
            return Err(crate::Error::msg("signal amplitude must be finite"));
        }
        let mut oscillator = ComplexOscillator::new(samp_rate, freq, -std::f32::consts::FRAC_PI_2)?;
        // The sine/-cosine waveform starts at sample one.
        let _ = oscillator.next();
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((
            Self {
                dst,
                amplitude,
                oscillator,
            },
            dr,
        ))
    }

    /// Change frequency in Hz, preserving the phase of the next sample.
    ///
    /// Rejects nonfinite values and nonpositive sample rates without changing
    /// the oscillator.
    pub fn set_frequency(&mut self, samp_rate: Float, freq: Float) -> Result<()> {
        self.oscillator.set_frequency(samp_rate, freq)
    }

    /// Set the phase in radians of the next generated sample.
    ///
    /// Rejects nonfinite phases without changing the oscillator.
    pub fn set_phase(&mut self, phase: Float) -> Result<()> {
        self.oscillator.set_phase(phase)
    }
}

impl Iterator for SignalSourceComplex {
    type Item = Complex;
    #[inline]
    fn next(&mut self) -> Option<Complex> {
        use crate::AlgebraicMul;
        self.oscillator
            .next()
            .map(|sample| sample.algebraic_mul(self.amplitude))
    }
}

impl Block for SignalSourceComplex {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut o = self.dst.write_buf()?;
        let n = o.len();
        for (to, from) in o.slice().iter_mut().zip(self.take(n)) {
            *to = from;
        }
        o.produce(n, &[]);
        Ok(BlockRet::WaitForStream(&self.dst, 1))
    }
}

/// Generate a pure real sine wave.
///
/// TODO: not an efficient implementation, and duplicates code with the Complex
/// version.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SignalSourceFloat {
    #[rustradio(out)]
    dst: WriteStream<Float>,
    amplitude: Float,
    rad_per_sample: f64,
    current: f64,
}

/// Generate pure complex sine sine.
impl SignalSourceFloat {
    /// Create a real sine source.
    ///
    /// Returns an error unless the sample rate is finite and positive and the
    /// frequency and amplitude are finite. Negative frequencies are supported.
    pub fn new(
        samp_rate: Float,
        freq: Float,
        amplitude: Float,
    ) -> Result<(Self, ReadStream<Float>)> {
        if !samp_rate.is_finite() || samp_rate <= 0.0 {
            return Err(crate::Error::msg(
                "signal sample rate must be finite and positive",
            ));
        }
        if !freq.is_finite() || !amplitude.is_finite() {
            return Err(crate::Error::msg(
                "signal frequency and amplitude must be finite",
            ));
        }
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((
            Self {
                dst,
                current: 0.0,
                amplitude,
                rad_per_sample: 2.0 * std::f64::consts::PI * f64::from(freq) / f64::from(samp_rate),
            },
            dr,
        ))
    }
}

impl Iterator for SignalSourceFloat {
    type Item = Float;
    fn next(&mut self) -> Option<Float> {
        self.current = (self.current + self.rad_per_sample) % (2.0 * std::f64::consts::PI);
        Some(self.amplitude * self.current.sin() as Float)
    }
}

impl Block for SignalSourceFloat {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut o = self.dst.write_buf()?;
        let n = o.len();
        o.slice()
            .iter_mut()
            .zip(self.take(n))
            .map(|(to, from)| {
                *to = from;
            })
            .for_each(drop);
        o.produce(n, &[]);
        Ok(BlockRet::WaitForStream(&self.dst, 1))
    }
}
/* vim: textwidth=80
 */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscillator::NORMALIZE_INTERVAL;

    fn expected_sample(rate: Float, freq: Float, amplitude: f32, index: usize) -> Complex {
        let phase = std::f64::consts::TAU * (f64::from(freq) % f64::from(rate)) / f64::from(rate)
            * index as f64;
        Complex::new(phase.sin() as f32, -phase.cos() as f32) * amplitude
    }

    #[test]
    fn complex_oscillator_matches_waveform_and_aliases() -> Result<()> {
        for freq in [
            0.0,
            100_000.0,
            -100_000.0,
            512_000.0,
            -512_000.0,
            1_124_000.0,
        ] {
            for amplitude in [0.0, 0.25, -2.0] {
                let (mut source, _output) = SignalSourceComplex::new(1_024_000.0, freq, amplitude)?;
                for index in 1..=2 * NORMALIZE_INTERVAL + 1 {
                    let got = source.next().unwrap();
                    let want = expected_sample(1_024_000.0, freq, amplitude, index);
                    assert!(
                        (got - want).norm() < 1e-6,
                        "{freq} Hz, sample {index}: {got} != {want}"
                    );
                }
            }
        }
        // Finite parameters can have extreme ratios; range reduction must
        // still leave a finite, unit-magnitude rotation.
        for freq in [f32::MAX, -f32::MAX] {
            let (mut source, _output) = SignalSourceComplex::new(f32::MIN_POSITIVE, freq, 1.0)?;
            for sample in source.by_ref().take(10) {
                assert!(sample.re.is_finite() && sample.im.is_finite());
                assert!((sample.norm() - 1.0).abs() < 1e-6);
            }
        }
        Ok(())
    }

    #[test]
    fn complex_oscillator_preserves_amplitude_and_phase_over_long_runs() -> Result<()> {
        for freq in [100_000.0, -100_000.0, 0.001] {
            let (mut source, _output) = SignalSourceComplex::new(1_024_000.0, freq, 1.0)?;
            for index in 1..=1_000_000 {
                let sample = source.next().unwrap();
                assert!((sample.norm() - 1.0).abs() < 2e-7);
                if index % NORMALIZE_INTERVAL == 0 {
                    let expected = expected_sample(1_024_000.0, freq, 1.0, index);
                    assert!((sample - expected).norm() < 1e-6);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn complex_oscillator_preserves_phase_under_backpressure() -> Result<()> {
        let (mut source, output) = SignalSourceComplex::new(1_024_000.0, 100_000.0, 1.0)?;
        assert!(matches!(source.work()?, BlockRet::WaitForStream(_, 1)));
        let phase = source.oscillator.phase();
        assert!(matches!(source.work()?, BlockRet::WaitForStream(_, 1)));
        assert_eq!(source.oscillator.phase(), phase);
        let (buffer, tags) = output.read_buf()?;
        let capacity = buffer.len();
        assert!(tags.is_empty());
        assert!(
            (buffer.slice()[0] - expected_sample(1_024_000.0, 100_000.0, 1.0, 1)).norm() < 1e-6
        );
        buffer.consume(13);
        assert!(matches!(source.work()?, BlockRet::WaitForStream(_, 1)));
        let (buffer, tags) = output.read_buf()?;
        assert_eq!(buffer.len(), capacity);
        assert!(tags.is_empty());
        for (offset, sample) in buffer.slice()[capacity - 13..].iter().enumerate() {
            let expected = expected_sample(1_024_000.0, 100_000.0, 1.0, capacity + offset + 1);
            assert!((*sample - expected).norm() < 1e-6);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_signal_parameters() {
        for rate in [0.0, -1.0, Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            assert!(SignalSourceFloat::new(rate, 1.0, 1.0).is_err());
            assert!(SignalSourceComplex::new(rate, 1.0, 1.0).is_err());
        }
        for invalid in [Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            assert!(SignalSourceFloat::new(10.0, invalid, 1.0).is_err());
            assert!(SignalSourceComplex::new(10.0, invalid, 1.0).is_err());
            assert!(SignalSourceFloat::new(10.0, 1.0, invalid).is_err());
            assert!(SignalSourceComplex::new(10.0, 1.0, invalid).is_err());
        }
    }

    #[test]
    fn supports_signed_frequency_and_zero_amplitude() -> Result<()> {
        let (mut positive, _) = SignalSourceComplex::new(16.0, 1.0, 1.0)?;
        let (mut negative, _) = SignalSourceComplex::new(16.0, -1.0, 1.0)?;
        let positive = positive.next().unwrap();
        let negative = negative.next().unwrap();
        assert!((positive.re + negative.re).abs() < 1e-6);
        assert!((positive.im - negative.im).abs() < 1e-6);
        let (mut silent, _) = SignalSourceFloat::new(16.0, -1.0, 0.0)?;
        assert_eq!(silent.next(), Some(0.0));
        Ok(())
    }

    #[test]
    fn explicit_phase_and_frequency_changes_use_the_shared_iterator() -> Result<()> {
        let (mut source, _output) = SignalSourceComplex::new(8.0, 2.0, 2.0)?;
        source.set_phase(0.0)?;
        assert_eq!(source.next(), Some(Complex::new(2.0, 0.0)));
        source.set_frequency(8.0, -2.0)?;
        assert!((source.next().unwrap() - Complex::new(0.0, 2.0)).norm() < 1e-6);
        assert!((source.next().unwrap() - Complex::new(2.0, 0.0)).norm() < 1e-6);
        Ok(())
    }
}
