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
        sample.norm_sqr()
    }
}
