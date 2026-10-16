//! IQ "balance" / DC offset removal for complex I/Q streams.
//!
//! Many SDR frontends show a big spike at DC (0 Hz) due to hardware/ADC
//! imperfections that add a constant bias to I and/or Q.
//!
//! This block tracks the running mean (DC component) of the complex stream with
//! a single-pole IIR low-pass filter and subtracts it from the input.
use crate::AlgebraicOps;
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Remove DC offset from a complex I/Q stream.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct IqBalance {
    alpha: Float,
    one_minus_alpha: Float,
    #[rustradio(default)]
    mean: Complex,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl IqBalance {
    /// Create a new IQ DC-removal block.
    ///
    /// `sample_rate` must be positive; the default time constant is 0.2 seconds.
    pub fn new(
        src: ReadStream<Complex>,
        sample_rate: u32,
    ) -> crate::Result<(Self, ReadStream<Complex>)> {
        // Default to tau found to work well with a rtl sdr
        Self::with_tau(src, sample_rate, 0.2)
    }

    /// Create a new IQ DC-removal block with an explicit time constant.
    ///
    /// `tau_seconds` is the approximate time constant of the mean estimator.
    /// Returns an error for zero sample rate or a nonfinite/nonpositive time
    /// constant.
    pub fn with_tau(
        src: ReadStream<Complex>,
        sample_rate: u32,
        tau_seconds: f64,
    ) -> crate::Result<(Self, ReadStream<Complex>)> {
        if sample_rate == 0 || !tau_seconds.is_finite() || tau_seconds <= 0.0 {
            return Err(crate::Error::msg(
                "IQ balance requires a positive sample rate and finite positive time constant",
            ));
        }
        let fs = f64::from(sample_rate);
        // Exponential smoothing coefficient: alpha = 1 - exp(-1/(tau*fs))
        let alpha = (1.0 - (-1.0 / (tau_seconds * fs)).exp()) as Float;
        Self::with_alpha(src, alpha)
    }

    /// Create a new IQ DC-removal block with explicit smoothing coefficient.
    ///
    /// `alpha` must be finite and in \([0, 1]\). Smaller values track DC slower.
    /// Invalid coefficients return an error instead of being clamped.
    pub fn with_alpha(
        src: ReadStream<Complex>,
        alpha: Float,
    ) -> crate::Result<(Self, ReadStream<Complex>)> {
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(crate::Error::msg(
                "IQ balance alpha must be finite and between zero and one",
            ));
        }
        let (dst, out) = crate::stream::new_stream()?;
        Ok((
            Self {
                alpha,
                one_minus_alpha: 1.0 - alpha,
                mean: Complex::default(),
                src,
                dst,
            },
            out,
        ))
    }

    fn process_sync(&mut self, x: Complex) -> Complex {
        use crate::algebraic_mul_complex_float as mul_c_f;
        // mean[n] = (1-a)*mean[n-1] + a*x[n]
        let a = mul_c_f(self.mean, self.one_minus_alpha);
        let b = mul_c_f(x, self.alpha);
        self.mean = a.algebraic_add(b);
        x - self.mean
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_invalid_settings_instead_of_substituting_defaults() {
        assert!(IqBalance::new(ReadStream::from_slice(&[]), 0).is_err());
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(IqBalance::with_tau(ReadStream::from_slice(&[]), 1000, invalid).is_err());
        }
        for invalid in [-0.1, 1.1, Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            assert!(IqBalance::with_alpha(ReadStream::from_slice(&[]), invalid).is_err());
        }
    }

    #[test]
    fn accepts_endpoint_coefficients() -> crate::Result<()> {
        for alpha in [0.0, 1.0] {
            let (mut block, output) =
                IqBalance::with_alpha(ReadStream::from_slice(&[Complex::new(1.0, -2.0)]), alpha)?;
            block.work()?;
            assert_eq!(
                output.read_buf()?.0.slice(),
                &[Complex::new(1.0 - alpha, -2.0 * (1.0 - alpha))]
            );
        }
        Ok(())
    }

    use super::*;
    use crate::block::Block;
    use crate::stream::ReadStream;

    #[test]
    fn removes_dc_offset_quickly_with_large_alpha() -> crate::Result<()> {
        let src = ReadStream::from_slice(&[Complex::new(1.0, -2.0); 8]);
        let (mut b, out) = IqBalance::with_alpha(src, 0.5)?;
        b.work()?;
        let (o, _) = out.read_buf()?;
        let s = o.slice();
        let last = s.last().copied().unwrap();
        assert!(last.re.abs() < 0.01, "unexpected residual I DC: {last:?}");
        assert!(last.im.abs() < 0.01, "unexpected residual Q DC: {last:?}");
        Ok(())
    }

    #[test]
    fn removes_dc_offset_quickly_with_huge_alpha() -> crate::Result<()> {
        let src = ReadStream::from_slice(&[Complex::new(1.0, -2.0); 8]);
        let (mut b, out) = IqBalance::with_alpha(src, 0.99)?;
        b.work()?;
        let (o, _) = out.read_buf()?;
        let s = o.slice();
        let last = s.last().copied().unwrap();
        assert!(last.re.abs() < 0.01, "unexpected residual I DC: {last:?}");
        assert!(last.im.abs() < 0.01, "unexpected residual Q DC: {last:?}");
        Ok(())
    }

    #[test]
    fn keeps_dc_offset_quickly_with_small_alpha() -> crate::Result<()> {
        let src = ReadStream::from_slice(&[Complex::new(1.0, -2.0); 8]);
        let (mut b, out) = IqBalance::with_alpha(src, 0.0000001)?;
        b.work()?;
        let (o, _) = out.read_buf()?;
        let s = o.slice();
        let last = s.last().copied().unwrap();
        assert!(
            last.re.abs() > 0.01,
            "unexpected non-residual I DC: {last:?}"
        );
        assert!(
            last.im.abs() > 0.01,
            "unexpected non-residual Q DC: {last:?}"
        );
        Ok(())
    }
}
