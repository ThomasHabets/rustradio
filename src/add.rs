//! Add two streams.
use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, ReadStream, ReadStreamp, Streamp};
use crate::Error;

/// Adds a constant value to every sample.
pub struct Add<T>
where
    T: Copy,
{
    a: ReadStreamp<T>,
    b: ReadStreamp<T>,
    dst: Streamp<T>,
}

impl<T> Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Create a new AddConst, providing the constant to be added.
    pub fn new(a: ReadStreamp<T>, b: ReadStreamp<T>) -> Self {
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
        let (a, tags) = self.a.read_buf()?;
        let (b, _tags) = self.b.read_buf()?;
        let n = std::cmp::min(a.len(), b.len());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        let n = std::cmp::min(n, o.len());
        let it = a.iter().zip(b.iter()).map(|(x, y)| *x + *y);
        for (w, samp) in o.slice().iter_mut().take(n).zip(it) {
            *w = samp;
        }
        a.consume(n);
        b.consume(n);
        o.produce(n, &tags);
        Ok(BlockRet::Ok)
    }
}
