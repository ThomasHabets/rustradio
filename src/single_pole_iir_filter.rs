//! Infinite Impulse Response (IIR) filter.
//!
//! Also see:
//! * <https://en.wikipedia.org/wiki/Infinite_impulse_response>
//! * <https://www.wavewalkerdsp.com/2022/08/10/single-pole-iir-filter-substitute-for-moving-average-filter/>
//! * [`iir_filter`](crate::iir_filter) module

use crate::stream::{ReadStream, WriteStream};
use crate::{Float, Sample};

struct SinglePoleIir<Tout> {
    alpha: Float, // TODO: GNURadio uses double
    one_minus_alpha: Float,
    prev_output: Tout,
}

impl<Tout> SinglePoleIir<Tout>
where
    Tout: Sample + std::ops::Mul<Float, Output = Tout> + std::ops::Add<Output = Tout>,
{
    fn new(alpha: Float) -> Option<Self> {
        let mut r = Self {
            alpha: Float::default(),
            one_minus_alpha: Float::default(),
            prev_output: Tout::default(),
        };
        r.set_taps(alpha)?;
        Some(r)
    }
    fn filter<Tin>(&mut self, sample: Tin) -> Tout
    where
        Tin: Sample + std::ops::Mul<Float, Output = Tin> + std::ops::Add<Tout, Output = Tout>,
    {
        let o: Tout = sample * self.alpha + self.prev_output * self.one_minus_alpha;
        self.prev_output = o;
        o
    }
    fn set_taps(&mut self, alpha: Float) -> Option<()> {
        if !(0.0..=1.0).contains(&alpha) {
            return None;
        }
        self.alpha = alpha;
        self.one_minus_alpha = 1.0 - alpha;
        Some(())
    }
}

/// Single pole IIR filter.
///
/// This can be used as a more efficient moving average.
///
/// See
/// <https://www.wavewalkerdsp.com/2022/08/10/single-pole-iir-filter-substitute-for-moving-average-filter/>
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct SinglePoleIirFilter<T>
where
    T: Sample + std::ops::Mul<Float, Output = T> + std::ops::Add<T, Output = T>,
{
    iir: SinglePoleIir<T>,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> SinglePoleIirFilter<T>
where
    T: Sample + std::ops::Mul<Float, Output = T> + std::ops::Add<T, Output = T>,
{
    /// Create a filter with `alpha` between zero and one, inclusive.
    ///
    /// # Errors
    ///
    /// Returns an error if `alpha` is outside that range or is not finite.
    pub fn new(src: ReadStream<T>, alpha: Float) -> crate::Result<(Self, ReadStream<T>)> {
        let iir = SinglePoleIir::<T>::new(alpha).ok_or_else(|| {
            crate::Error::msg(format!(
                "SinglePoleIirFilter alpha must be finite and between 0 and 1, got {alpha}"
            ))
        })?;
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((Self { src, dst, iir }, dr))
    }
    fn process_sync(&mut self, a: T) -> T {
        self.iir.filter(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::{Complex, Result};

    #[test]
    fn iir_ff() -> Result<()> {
        // TODO: create an actual test.
        let src = ReadStream::from_slice(&[0.1, 0.2]);
        let (mut iir, _) = SinglePoleIirFilter::new(src, 0.2)?;
        iir.work()?;
        Ok(())
    }

    #[test]
    fn iir_cc() -> Result<()> {
        // TODO: create an actual test.
        let src = ReadStream::from_slice(&[Complex::new(1.0, 0.1), Complex::default()]);
        let (mut iir, _) = SinglePoleIirFilter::new(src, 0.2)?;
        iir.work()?;
        Ok(())
    }

    #[test]
    fn reject_bad_alpha() -> Result<()> {
        for tv in [0.0, 0.1, 1.0] {
            let src = ReadStream::from_slice(&[0.1, 0.2]);
            SinglePoleIirFilter::new(src, tv)?;
        }
        for alpha in [-0.1, 1.1, Float::NAN, Float::INFINITY, Float::NEG_INFINITY] {
            let src = ReadStream::from_slice(&[0.1, 0.2]);
            let err = SinglePoleIirFilter::new(src, alpha)
                .err()
                .expect("invalid alpha must be rejected");
            assert!(
                err.to_string()
                    .contains("alpha must be finite and between 0 and 1")
            );
        }
        Ok(())
    }
}
