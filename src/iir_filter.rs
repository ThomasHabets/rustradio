/*! Infinite impulse response filter

*/
use std::collections::VecDeque;

use crate::Float;

/// Ability to call .min and .max, like floats.
pub trait MinMax {
    /// Return min of two values.
    fn min(&self, o: Self) -> Self;

    /// Return max of two values.
    fn max(&self, o: Self) -> Self;
}
impl MinMax for Float {
    fn max(&self, r: Float) -> Self {
        r.max(*self)
    }
    fn min(&self, r: Float) -> Self {
        r.min(*self)
    }
}

/// General filter.
///
/// TODO: also add filter_n.
pub trait Filter<T: Copy + Default>: Send {
    /// Filter from one input sample.
    fn filter(&mut self, input: T) -> T;

    /// Fill filter history.
    fn fill(&mut self, s: T);
}

/// General filter.
///
/// TODO: also add filter_n.
pub trait CappedFilter<T: Copy + Default + MinMax>: Filter<T> {
    /// Filter from one input sample.
    fn filter_capped(&mut self, input: T, mi: T, mx: T) -> T;
}

/// Finite impulse response filter.
pub struct IirFilter<T: Copy> {
    taps: Vec<T>,
    buf: VecDeque<T>,
}

impl<T> IirFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    /// Create new IIR.
    pub fn new(taps: &[T]) -> Self {
        Self {
            taps: taps.to_vec(),
            buf: VecDeque::new(),
        }
    }
}

impl<T> Filter<T> for IirFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T> + Send,
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

impl<T> CappedFilter<T> for IirFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T> + MinMax + Send,
{
    fn filter_capped(&mut self, sample: T, mi: T, mx: T) -> T {
        let mut ret = self.taps[0] * sample;
        for (i, s) in self.buf.iter().rev().enumerate() {
            ret = ret + *s * self.taps[i + 1];
        }
        ret = ret.min(mx).max(mi);
        self.buf.push_back(ret);
        if self.buf.len() == self.taps.len() {
            self.buf.pop_front();
        }
        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

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
}
