//! Infinite Impulse Response (IIR) filter.
use anyhow::Result;

use crate::Float;
use crate::stream::{ReadStream, WriteStream};

struct SinglePoleIIR<Tout> {
    alpha: Float, // TODO: GNURadio uses double
    one_minus_alpha: Float,
    prev_output: Tout,
}

impl<Tout> SinglePoleIIR<Tout>
where
    Tout: Copy + Default + std::ops::Mul<Float, Output = Tout> + std::ops::Add<Output = Tout>,
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
        Tin: Copy + std::ops::Mul<Float, Output = Tin> + std::ops::Add<Tout, Output = Tout>,
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

/// Infinite Impulse Response (IIR) filter.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct SinglePoleIIRFilter<T>
where
    T: Copy
        + Default
        + std::ops::Mul<Float, Output = T>
        + std::ops::Mul<T, Output = T>
        + std::ops::Add<T, Output = T>,
{
    iir: SinglePoleIIR<T>,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> SinglePoleIIRFilter<T>
where
    T: Copy
        + Default
        + std::ops::Mul<Float, Output = T>
        + std::ops::Mul<T, Output = T>
        + std::ops::Add<T, Output = T>,
{
    /// Create new IIR filter.
    // TODO: have it take IIR, so that we can generate new()?
    pub fn new(src: ReadStream<T>, alpha: Float) -> Option<(Self, ReadStream<T>)> {
        let (dst, dr) = crate::stream::new_stream();
        Some((
            Self {
                src,
                dst,
                iir: SinglePoleIIR::<T>::new(alpha)?,
            },
            dr,
        ))
    }
    fn process_sync(&mut self, a: T) -> T {
        self.iir.filter(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::{Complex, Error};

    #[test]
    fn iir_ff() -> Result<()> {
        // TODO: create an actual test.
        let src = ReadStream::from_slice(&[0.1, 0.2]);
        let (mut iir, _) =
            SinglePoleIIRFilter::new(src, 0.2).ok_or(Error::new("alpha out of range"))?;
        iir.work()?;
        Ok(())
    }

    #[test]
    fn iir_cc() -> Result<()> {
        // TODO: create an actual test.
        let src = ReadStream::from_slice(&[Complex::new(1.0, 0.1), Complex::default()]);
        let (mut iir, _) =
            SinglePoleIIRFilter::new(src, 0.2).ok_or(Error::new("alpha out of range"))?;
        iir.work()?;
        Ok(())
    }

    #[test]
    fn reject_bad_alpha() -> Result<()> {
        for tv in [0.0, 0.1, 1.0] {
            let src = ReadStream::from_slice(&[0.1, 0.2]);
            SinglePoleIIRFilter::new(src, tv).ok_or(Error::new("should accept {tv}"))?;
        }
        let src = ReadStream::from_slice(&[0.1, 0.2]);
        if SinglePoleIIRFilter::new(src, -0.1).is_some() {
            return Err(Error::new("should not accept -0.1").into());
        }
        let src = ReadStream::from_slice(&[0.1, 0.2]);
        if SinglePoleIIRFilter::new(src, 1.1).is_some() {
            return Err(Error::new("should not accept 1.1").into());
        }
        Ok(())
    }
}
