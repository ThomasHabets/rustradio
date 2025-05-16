//! Blocks for making simple synchronous conversions/mappings.

use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float, Sample};

/// Arbitrary mapping using a lambda.
///
/// A Map block transforms one sample at a time, from input to output. The input
/// and output can be different types.
///
/// If there's more than one input or output stream, then you have to make a
/// dedicated block, like [`FloatToComplex`].
///
/// ```
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::Map;
/// let (src_block, src) = ConstantSource::new(Complex::new(12.0, 13.0));
/// let (b, out) = Map::new(src, "mymap", move |x| x.re + 10.0);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, custom_name, sync, new)]
pub struct Map<In, Out, F>
where
    In: Sample,
    Out: Sample,
    F: Fn(In) -> Out,
{
    #[rustradio(into)]
    name: String,
    map: F,
    #[rustradio(in)]
    src: ReadStream<In>,
    #[rustradio(out)]
    dst: WriteStream<F::Output>,
}

impl<In, Out, F> Map<In, Out, F>
where
    In: Sample,
    Out: Sample,
    F: Fn(In) -> Out,
{
    fn process_sync(&mut self, s: In) -> Out {
        (self.map)(s)
    }
    /// Name of the block.
    pub fn custom_name(&self) -> &str {
        &self.name
    }
}

/// Convert two floats stream to one complex stream.
///
/// ```
/// use rustradio::blocks::ConstantSource;
/// use rustradio::blocks::FloatToComplex;
/// let (re_block, re_src) = ConstantSource::new(1.0);
/// let (im_block, im_src) = ConstantSource::new(42.0);
/// let (b, out) = FloatToComplex::new(re_src, im_src);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct FloatToComplex {
    #[rustradio(in)]
    re: ReadStream<Float>,
    #[rustradio(in)]
    im: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl FloatToComplex {
    fn process_sync(&self, re: Float, im: Float) -> Complex {
        Complex::new(re, im)
    }
}

/// Convert a complex stream to two float streams.
///
/// If only one of the two streams are needed, then it's better to use a [`Map`]
/// block.
///
/// ```
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::blocks::ComplexToFloat;
/// let (src_block, src) = ConstantSource::new(Complex::new(1.0, 2.0));
/// let (b, out_re, out_im) = ComplexToFloat::new(src);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct ComplexToFloat {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    re: WriteStream<Float>,
    #[rustradio(out)]
    im: WriteStream<Float>,
}

impl ComplexToFloat {
    fn process_sync(&self, c: Complex) -> (Float, Float) {
        (c.re, c.im)
    }
}
