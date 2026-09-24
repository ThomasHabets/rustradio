//! Convert Complex numbers to square of their magnitude.
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Convert Complex numbers to square of their magnitude.
///
/// `out = in.re ^ 2 + in.im ^ 2`
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct ComplexToMag2 {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
}

impl ComplexToMag2 {
    fn process_sync(&self, sample: Complex) -> Float {
        sample
            .re
            .algebraic_mul(sample.re)
            .algebraic_add(sample.im.algebraic_mul(sample.im))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    #[test]
    fn magnitudes() -> crate::Result<()> {
        let input = [
            Complex::new(0.0, 0.0),
            Complex::new(3.0, 4.0),
            Complex::new(-3.0, 4.0),
            Complex::new(3.0, -4.0),
            Complex::new(-3.0, -4.0),
            Complex::new(0.0, -2.0),
            Complex::new(0.5, 0.0),
        ];
        let (mut block, out) = ComplexToMag2::new(ReadStream::from_slice(&input));
        block.work()?;
        assert_eq!(
            out.read_buf()?.0.slice(),
            &[0.0, 25.0, 25.0, 25.0, 25.0, 4.0, 0.25]
        );
        Ok(())
    }
}
