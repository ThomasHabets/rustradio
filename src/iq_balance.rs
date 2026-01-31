//! IQ "balance" / DC offset removal for complex I/Q streams.
//!
//! Many SDR frontends show a big spike at DC (0 Hz) due to hardware/ADC
//! imperfections that add a constant bias to I and/or Q.
//!
//! This block tracks the running mean (DC component) of the complex stream with
//! a single-pole IIR low-pass filter and subtracts it from the input.
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
    /// `sample_rate` is used to derive a sane default time constant.
    #[must_use]
    pub fn new(src: ReadStream<Complex>, sample_rate: u32) -> (Self, ReadStream<Complex>) {
        // Default to tau found to work well with a rtl sdr
        Self::with_tau(src, sample_rate, 0.2)
    }

    /// Create a new IQ DC-removal block with an explicit time constant.
    ///
    /// `tau_seconds` is the approximate time constant of the mean estimator.
    #[must_use]
    pub fn with_tau(
        src: ReadStream<Complex>,
        sample_rate: u32,
        tau_seconds: f64,
    ) -> (Self, ReadStream<Complex>) {
        let fs = sample_rate.max(1) as f64;
        let tau = if tau_seconds.is_finite() && tau_seconds > 0.0 {
            tau_seconds
        } else {
            0.5
        };
        // Exponential smoothing coefficient: alpha = 1 - exp(-1/(tau*fs))
        let alpha64 = 1.0 - (-1.0 / (tau * fs)).exp();
        let alpha = alpha64.clamp(0.0, 1.0) as Float;
        Self::with_alpha(src, alpha)
    }

    /// Create a new IQ DC-removal block with explicit smoothing coefficient.
    ///
    /// `alpha` must be in \([0, 1]\). Smaller values track DC slower.
    #[must_use]
    pub fn with_alpha(src: ReadStream<Complex>, alpha: Float) -> (Self, ReadStream<Complex>) {
        let alpha = alpha.clamp(0.0, 1.0);
        let (dst, out) = crate::stream::new_stream();
        (
            Self {
                alpha,
                one_minus_alpha: 1.0 - alpha,
                mean: Complex::default(),
                src,
                dst,
            },
            out,
        )
    }

    fn process_sync(&mut self, x: Complex) -> Complex {
        // mean[n] = (1-a)*mean[n-1] + a*x[n]
        self.mean = self.mean * self.one_minus_alpha + x * self.alpha;
        x - self.mean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::stream::ReadStream;

    #[test]
    fn removes_dc_offset_quickly_with_large_alpha() -> crate::Result<()> {
        let src = ReadStream::from_slice(&[Complex::new(1.0, -2.0); 8]);
        let (mut b, out) = IqBalance::with_alpha(src, 0.5);
        b.work()?;
        let (o, _) = out.read_buf()?;
        let s = o.slice();
        let last = s.last().copied().unwrap();
        assert!(last.re.abs() < 0.01, "unexpected residual I DC: {last:?}");
        assert!(last.im.abs() < 0.01, "unexpected residual Q DC: {last:?}");
        Ok(())
    }
}
