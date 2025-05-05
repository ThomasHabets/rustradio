//! Blocks for converting from one type to another.

use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Builder for Map.
///
/// ```
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::Map;
/// let (src_block, src) = ConstantSource::new(13);
/// let (b, out) = Map::builder(src, move |x| x + 10)
///   .name("mymap")
///   .build();
/// ```
pub struct MapBuilder<In, Out, F>
where
    F: Fn(In) -> Out,
{
    map: F,
    name: String,
    src: ReadStream<In>,
}

impl<In, Out, F> MapBuilder<In, Out, F>
where
    In: Copy,
    Out: Copy,
    F: Fn(In) -> Out,
{
    /// Set name.
    pub fn name<T: Into<String>>(mut self, name: T) -> MapBuilder<In, Out, F> {
        self.name = name.into();
        self
    }
    /// Build Map.
    pub fn build(self) -> (Map<In, Out, F>, ReadStream<Out>) {
        Map::new(self.name, self.src, self.map)
    }
}

/// Arbitrary mapping.
///
/// Use [`MapBuilder`] to create a `Map` block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, custom_name, sync)]
pub struct Map<In, Out, F>
where
    In: Copy,
    Out: Copy,
    F: Fn(In) -> Out,
{
    name: String,
    map: F,
    #[rustradio(in)]
    src: ReadStream<In>,
    #[rustradio(out)]
    dst: WriteStream<F::Output>,
}

impl<In, Out, F> Map<In, Out, F>
where
    In: Copy,
    Out: Copy,
    F: Fn(In) -> Out,
{
    /// Create new MapBuilder.
    pub fn builder(src: ReadStream<In>, map: F) -> MapBuilder<In, Out, F> {
        MapBuilder {
            src,
            map,
            name: "Map".into(),
        }
    }
    /// Create new Map block.
    ///
    /// A Map block transforms one sample at a time, from input to output.
    ///
    /// If there's more than one input or output stream, then you have to make a
    /// dedicated block.
    fn new(name: String, src: ReadStream<In>, map: F) -> (Self, ReadStream<Out>) {
        let dst = crate::stream::new_stream();
        (
            Self {
                name,
                map,
                src,
                dst: dst.0,
            },
            dst.1,
        )
    }
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
