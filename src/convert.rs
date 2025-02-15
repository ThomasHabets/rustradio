//! Blocks for converting from one type to another.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::Error;
use crate::{Complex, Float};

/// Builder for Map.
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
    /// Create new MapBuilder.
    pub fn new(src: ReadStream<In>, map: F) -> Self {
        Self {
            src,
            map,
            name: "Map".into(),
        }
    }
    /// Set name.
    pub fn name(mut self, name: String) -> MapBuilder<In, Out, F> {
        self.name = name;
        self
    }
    /// Build Map.
    pub fn build(self) -> (Map<In, Out, F>, ReadStream<Out>) {
        Map::new(self.name, self.src, self.map)
    }
}

/// Arbitrary mapping
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
    /// Create new FloatToU32, scaled.
    ///
    /// Return value is the input multiplied by the scale. E.g. with a
    /// scale of 100.0, the input 0.123 becomes 12.
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

/// Convert floats to complex.
// TODO: make sync.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct FloatToComplex {
    #[rustradio(in)]
    re: ReadStream<Float>,
    #[rustradio(in)]
    im: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl Block for FloatToComplex {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (a, tags) = self.re.read_buf()?;
        if a.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.re, 1));
        }
        let (b, _) = self.im.read_buf()?;
        if b.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.im, 1));
        }
        let n = std::cmp::min(a.len(), b.len());
        let mut o = self.dst.write_buf()?;
        let n = std::cmp::min(n, o.len());
        o.fill_from_iter(
            a.iter()
                .zip(b.iter())
                .take(n)
                .map(|(x, y)| Complex::new(*x, *y)),
        );
        a.consume(n);
        b.consume(n);
        o.produce(n, &tags);
        Ok(BlockRet::Ok)
    }
}
