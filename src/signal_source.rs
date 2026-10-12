//! Generate a pure signal.
use crate::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Generate a pure complex sine wave.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SignalSourceComplex {
    #[rustradio(out)]
    dst: WriteStream<Complex>,

    amplitude: Float,
    rad_per_sample: f64,
    current: f64,
}

/// Generate pure complex sine sine.
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

impl Iterator for SignalSourceComplex {
    type Item = Complex;
    fn next(&mut self) -> Option<Complex> {
        self.current = (self.current + self.rad_per_sample) % (2.0 * std::f64::consts::PI);
        Some(
            self.amplitude
                * Complex::new(
                    self.current.sin() as Float,
                    (self.current - std::f64::consts::PI / 2.0).sin() as Float,
                ),
        )
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
}
