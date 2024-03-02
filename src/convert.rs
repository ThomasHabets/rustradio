//! Blocks for converting from one type to another.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
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
pub struct Map<In, Out, F>
where
    F: Fn(In) -> Out,
{
    name: String,
    map: F,
    src: Streamp<In>,
    dst: Streamp<F::Output>,
}

impl<In, Out, F> Map<In, Out, F>
where
    F: Fn(In) -> Out,
{
    /// Return the output stream.
    pub fn out(&self) -> Streamp<Out> {
        self.dst.clone()
    }
    /// Create new FloatToU32, scaled.
    ///
    /// Return value is the input multiplied by the scale. E.g. with a
    /// scale of 100.0, the input 0.123 becomes 12.
    fn new(name: String, src: Streamp<In>, map: F) -> Self {
        Self {
            name,
            map,
            src,
            dst: new_streamp(),
        }
    }
    fn process_one(&mut self, s: In) -> Out {
        (self.map)(s)
    }
    /// Name of the block.
    pub fn name(&self) -> &str {
        &self.name
    }
}
impl<In, Out, F> Block for Map<In, Out, F>
where
    In: Copy,
    Out: Copy,
    F: Fn(In) -> Out,
{
    fn block_name(&self) -> &str {
        &self.name
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        // Bindings, since borrow checker won't let us call
        // mut `process_one` if we borrow `src` and `dst`.
        let ibind = self.src.clone();
        let obind = self.dst.clone();

        // Get input and output buffers.
        let (i, tags) = ibind.read_buf()?;
        let mut o = obind.write_buf()?;

        // Don't process more than we have, and fit.
        let n = std::cmp::min(i.len(), o.len());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }

        // Map one sample at a time. Is this really the best way?
        for (place, ival) in o.slice().iter_mut().zip(i.iter()) {
            *place = self.process_one(*ival);
        }

        // Finalize.
        o.produce(n, &tags);
        i.consume(n);
        Ok(BlockRet::Ok)
    }
}

/// Convert floats to complex.
pub struct FloatToComplex {
    re: Streamp<Float>,
    im: Streamp<Float>,
    dst: Streamp<Complex>,
}

impl FloatToComplex {
    /// Create new block.
    pub fn new(re: Streamp<Float>, im: Streamp<Float>) -> Self {
        Self {
            re,
            im,
            dst: new_streamp(),
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<Complex> {
        self.dst.clone()
    }
}

impl Block for FloatToComplex {
    fn block_name(&self) -> &str {
        "FloatToComplex"
    }
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
