//! Add two streams.
use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
use crate::Error;

/// Adds a constant value to every sample.
pub struct Add<T>
where
    T: Copy,
{
    a: Streamp<T>,
    b: Streamp<T>,
    dst: Streamp<T>,
}

impl<T> Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Create a new AddConst, providing the constant to be added.
    pub fn new(a: Streamp<T>, b: Streamp<T>) -> Self {
        Self {
            a,
            b,
            dst: new_streamp(),
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T> Block for Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    fn block_name(&self) -> &'static str {
        "Add"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut a = self.a.lock()?;
        let mut b = self.b.lock()?;
        let n = std::cmp::min(a.available(), b.available());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.lock()?;
        o.write(a.iter().zip(b.iter()).take(n).map(|(x, y)| *x + *y));
        a.consume(n);
        b.consume(n);
        Ok(BlockRet::Ok)
    }
}
