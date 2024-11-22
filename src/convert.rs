//! Blocks for converting from one type to another.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;
use crate::{Complex, Float};

/// Builder for Map.
pub struct MapBuilder<In, Out, F>
where
    F: Fn(In) -> Out,
{
    map: F,
    name: String,
    src: Streamp<In>,
}

impl<In, Out, F> MapBuilder<In, Out, F>
where
    In: Copy,
    Out: Copy,
    F: Fn(In) -> Out,
{
    /// Create new MapBuilder.
    pub fn new(src: Streamp<In>, map: F) -> Self {
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
    pub fn build(self) -> Map<In, Out, F> {
        Map::new(self.name, self.src, self.map)
    }
}

/// Arbitrary mapping
#[derive(rustradio_macros::Block)]
#[rustradio(crate, out, custom_name, sync)]
pub struct Map<In, Out, F>
where
    In: Copy,
    Out: Copy,
    F: Fn(In) -> Out,
{
    name: String,
    map: F,
    #[rustradio(in)]
    src: Streamp<In>,
    #[rustradio(out)]
    dst: Streamp<F::Output>,
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
    fn new(name: String, src: Streamp<In>, map: F) -> Self {
        Self {
            name,
            map,
            src,
            dst: Stream::newp(),
        }
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
#[rustradio(crate, out, new)]
pub struct FloatToComplex {
    #[rustradio(in)]
    re: Streamp<Float>,
    #[rustradio(in)]
    im: Streamp<Float>,
    #[rustradio(out)]
    dst: Streamp<Complex>,
}

impl Block for FloatToComplex {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (a, tags) = self.re.read_buf()?;
        let (b, _) = self.im.read_buf()?;
        let n = std::cmp::min(a.len(), b.len());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
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
