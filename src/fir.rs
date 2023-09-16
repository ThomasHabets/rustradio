/*
 * TODO:
 * * Only handles case where input, output, and tap type are all the same.
 */

use anyhow::Result;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;

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
        assert_almost_equal(
            &filter.filter_n(&input),
            &[
                Complex::new(2.3, 0.22),
                Complex::new(3.41, 0.6),
                Complex::new(4.56, 0.6),
                Complex::new(5.6, 0.84),
            ],
        );
    }

    fn assert_almost_equal(left: &[Complex], right: &[Complex]) {
        assert_eq!(
            left.len(),
            right.len(),
            "\nleft: {:?}\nright: {:?}",
            left,
            right
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).norm_sqr();
            if dist > 0.001 {
                assert_eq!(left[i], right[i], "\nleft: {:?}\nright: {:?}", left, right);
            }
        }
    }
}

use crate::{Block, StreamReader, StreamWriter};

pub struct FIR<T> {
    taps: Vec<T>,
}

impl<T> FIR<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    pub fn new(taps: &[T]) -> Self {
        Self {
            taps: taps.iter().copied().rev().collect(),
        }
    }
    pub fn filter(&self, input: &[T]) -> T {
        input
            .iter()
            .take(self.taps.len())
            .enumerate()
            .fold(T::default(), |acc, (i, x)| acc + *x * self.taps[i])
    }
    pub fn filter_n(&self, input: &[T]) -> Vec<T> {
        let n = input.len() - self.taps.len() + 1;
        (0..n).map(|i| self.filter(&input[i..])).collect()
    }
}

pub struct FIRFilter<T> {
    fir: FIR<T>,
}

impl<T> FIRFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    pub fn new(taps: &[T]) -> Self {
        Self {
            fir: FIR::new(taps),
        }
    }
}

impl<T> Block<T, T> for FIRFilter<T>
where
    T: Copy + Default + std::ops::Mul<T, Output = T> + std::ops::Add<T, Output = T>,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let n = std::cmp::min(r.available(), w.capacity());
        if n > 0 {
            w.write(&self.fir.filter_n(&r.buffer()[..n]))?;
            r.consume(n);
        }
        Ok(())
    }
}
