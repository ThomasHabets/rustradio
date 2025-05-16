//! Infinite impulse response filter.
//!
//! This module doesn't contain any blocks. It only has the IIR specific code.
//! Although when an IIR filter block is written, this module is likely where
//! it'll end up.
//!
//! For blocks, see [`SinglePoleIirFilter`](crate::blocks::SinglePoleIirFilter).
use std::collections::VecDeque;

use crate::{Float, Sample};

/// Ability to call `.clamp()`.
///
/// Needed for `ClampedFilter`.
pub trait Clamp {
    /// Return clamped value.
    fn clamp(&self, mi: Self, mx: Self) -> Self;
}
impl Clamp for Float {
    fn clamp(&self, mi: Float, mx: Float) -> Self {
        Float::clamp(*self as Float, mi, mx)
    }
}

/// General IIR filter.
///
/// TODO: also add filter_n?
pub trait Filter<T: Sample>: Send {
    /// Filter from one input sample.
    fn filter(&mut self, input: T) -> T;

    /// Fill filter history with the given value.
    fn fill(&mut self, s: T);
}

/// A ClampedFilter is like a regular filter, but clamps the output value to be
/// between the minimum and the maximum.
///
/// TODO: also add filter_n?
pub trait ClampedFilter<T: Sample + Clamp>: Filter<T> {
    /// Filter from one input sample, but with clamped output.
    fn filter_clamped(&mut self, input: T, mi: T, mx: T) -> T;
}

/// Finite impulse response filter.
///
/// An IIR filter is like a FIR but feeds back the output, meaning while
/// intended to dampen, it never full loses its "history". Hence "infinite".
///
/// IIR filters are a bit more complicated than FIR filters, but can also be
/// more efficient.
///
/// For more info see <https://en.wikipedia.org/wiki/Infinite_impulse_response>.
pub struct IirFilter<T: Sample> {
    taps: Vec<T>,
    buf: VecDeque<T>,
}

impl<T> IirFilter<T>
where
    T: Sample + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    /// Create new IIR from the provided taps.
    pub fn new(taps: &[T]) -> Self {
        Self {
            taps: taps.to_vec(),
            buf: VecDeque::new(),
        }
    }
}

impl<T> Filter<T> for IirFilter<T>
where
    T: Sample
        + std::ops::Mul<T, Output = T>
        + std::ops::Add<T, Output = T>
        + Send
        + std::fmt::Debug,
{
    fn filter(&mut self, sample: T) -> T {
        let mut ret = self.taps[0] * sample;
        for (i, s) in self.buf.iter().rev().enumerate() {
            ret = ret + *s * self.taps[i + 1];
        }
        self.buf.push_back(ret);
        if self.buf.len() == self.taps.len() {
            self.buf.pop_front();
        }
        ret
    }

    fn fill(&mut self, s: T) {
        for _ in 0..(self.taps.len() - 1) {
            self.buf.push_back(s);
        }
    }
}

impl<T> ClampedFilter<T> for IirFilter<T>
where
    T: Sample
        + std::ops::Mul<T, Output = T>
        + std::ops::Add<T, Output = T>
        + Clamp
        + Send
        + std::fmt::Debug,
{
    fn filter_clamped(&mut self, sample: T, mi: T, mx: T) -> T {
        let mut ret = self.taps[0] * sample;
        for (i, s) in self.buf.iter().rev().enumerate() {
            ret = ret + *s * self.taps[i + 1];
        }
        ret = ret.clamp(mi, mx);
        self.buf.push_back(ret);
        if self.buf.len() == self.taps.len() {
            self.buf.pop_front();
        }
        ret
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn zero_pole() -> Result<()> {
        let mut f = IirFilter::new(&[1.0]);
        assert_eq!(f.filter(123.0), 123.0);
        assert_eq!(f.filter(123.0), 123.0);
        let mut f = IirFilter::new(&[-0.5]);
        assert_eq!(f.filter(402.0), -201.0);
        assert_eq!(f.filter(402.0), -201.0);
        Ok(())
    }

    #[test]
    fn single_pole() -> Result<()> {
        let mut f = IirFilter::new(&[1.0, 0.0]);
        assert_eq!(f.filter(10.0), 10.0);
        assert_eq!(f.filter(10.0), 10.0);
        assert_eq!(f.filter(10.0), 10.0);

        let mut f = IirFilter::new(&[0.9f32, 0.1]);
        assert_eq!(f.filter(100.0), 90.0);
        assert_eq!(f.filter(100.0), 99.0);
        assert_eq!(f.filter(100.0), 99.9);
        assert!((f.filter(100.0) - 99.99).abs() < 0.00001);

        Ok(())
    }

    #[test]
    fn single_pole_clamped() -> Result<()> {
        let mut f = IirFilter::new(&[1.0, 0.0]);
        assert_eq!(f.filter_clamped(10.0, 0.0, 1.0), 1.0);
        assert_eq!(f.filter_clamped(10.0, 0.0, 1.0), 1.0);
        assert_eq!(f.filter_clamped(10.0, 0.0, 1.0), 1.0);

        Ok(())
    }

    #[test]
    fn multi_pole() -> Result<()> {
        let mut f = IirFilter::new(&[1.0, 0.0, 0.0]);
        assert_eq!(f.filter(10.0), 10.0);
        assert_eq!(f.filter(10.0), 10.0);
        assert_eq!(f.filter(10.0), 10.0);
        assert_eq!(f.filter(10.0), 10.0);

        let mut f = IirFilter::new(&[1.0f32, 0.9, 0.1]);
        assert_eq!(f.filter(100.0), 100.0);
        assert_eq!(f.filter(100.0), 190.0);
        assert_eq!(f.filter(100.0), 281.0);
        assert_eq!(f.filter(100.0), 371.9);

        Ok(())
    }

    #[test]
    fn filled() -> Result<()> {
        let mut f = IirFilter::new(&[1.0f32, 0.9, 0.1]);
        f.fill(100.0);
        assert_eq!(f.filter(100.0), 200.0);
        assert_eq!(f.filter(100.0), 290.0);
        assert_eq!(f.filter(200.0), 481.0);
        Ok(())
    }
}
