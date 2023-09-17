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

    #[test]
    fn test_filter_generator() {
        let taps = low_pass(10000.0, 1000.0, 1000.0);
        assert_eq!(taps.len(), 25);
        assert_eq!(
            taps,
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
                Complex::new(0.002010403, 0.0)
            ]
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

use crate::{Block, Complex, Float, StreamReader, StreamWriter};

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

// TODO: this would be faster if we supported filtering a Complex by a Float.
pub fn low_pass(samp_rate: Float, cutoff: Float, twidth: Float) -> Vec<Complex> {
    let pi = std::f64::consts::PI as Float;
    let ntaps = {
        let a: Float = 53.0; // Hamming.
        let t = (a * samp_rate / (22.0 * twidth)) as usize;
        if (t & 1) == 0 {
            t + 1
        } else {
            t
        }
    };
    let mut taps = vec![Float::default(); ntaps];
    let window: Vec<Float> = {
        // Hamming
        let m = (ntaps - 1) as Float;
        (0..ntaps)
            .map(|n| 0.54 - 0.46 * (2.0 * pi * (n as Float) / m).cos())
            .collect()
    };
    let m = (ntaps - 1) / 2;
    let fwt0 = 2.0 * pi * cutoff / samp_rate;
    for nm in 0..ntaps {
        let n = nm as i64 - m as i64;
        let nf = n as Float;
        taps[nm] = if n == 0 {
            fwt0 / pi * window[nm]
        } else {
            ((nf * fwt0).sin() / (nf * pi)) * window[nm]
        };
    }
    let gain = {
        let gain: Float = 1.0;
        let mut fmax = taps[m];
        for n in 1..=m {
            fmax += 2.0 * taps[n + m];
        }
        gain / fmax
    };
    taps.into_iter()
        .map(|t| Complex::new(t * gain, 0.0))
        .collect()
}
