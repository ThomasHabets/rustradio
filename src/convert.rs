//! Blocks for converting from one type to another.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, ReadStream, ReadStreamp, Streamp};
use crate::Error;
use crate::{map_block_convert_macro, Complex, Float};

/// Convert floats to unsigned 32bit int, scaled if needed.
///
/// `u32 = Float * scale`
pub struct FloatToU32 {
    scale: Float,
    src: ReadStreamp<Float>,
    dst: Streamp<u32>,
}

impl FloatToU32 {
    /// Create new FloatToU32, scaled.
    ///
    /// Return value is the input multiplied by the scale. E.g. with a
    /// scale of 100.0, the input 0.123 becomes 12.
    pub fn new(src: ReadStreamp<Float>, scale: Float) -> Self {
        Self {
            scale,
            src,
            dst: new_streamp(),
        }
    }
    fn process_one(&mut self, s: Float) -> u32 {
        (s * self.scale) as u32
    }
}
map_block_convert_macro![FloatToU32, u32];

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
    fn block_name(&self) -> &'static str {
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

/*
struct Convert<From, To> {
    scale_from: From,
    scale_to: To,
}
impl std::convert::Into<u32> for Float {
    fn into(t: Float) -> u32 {
        t as u32
    }
}
impl<From, To> Convert<From, To>
where From: std::ops::Mul<Output=From> + std::convert::TryInto<To>,
      To: std::ops::Mul<Output=To>
{
    fn new(scale_from: From, scale_to: To) -> Self {
        Self{
            scale_from,
            scale_to,
        }
    }
    pub fn work(&mut self, r: &mut Stream<From>, w: &mut Stream<To>) -> Result<()>
    where <From as TryInto<To>>::Error: std::fmt::Debug
    {
        let v = r.data.iter().map(|e| {
            //From::into(*e * self.scale_from) * self.scale_to
            (*e * self.scale_from).try_into().unwrap() * self.scale_to
        });
        Ok(())
    }
}
*/
