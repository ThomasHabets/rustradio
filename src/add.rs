//! Add two streams.
use crate::block::{AutoBlock, Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

use rustradio_macros::Eof;

/// Adds two streams, sample wise.
#[derive(Eof)]
pub struct Add<T>
where
    T: Copy,
{
    /// Hello world.
    #[rustradio(in)]
    a: Streamp<T>,

    #[rustradio(in)]
    b: Streamp<T>,

    #[rustradio(in)]
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
            dst: Stream::newp(),
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
    fn block_name(&self) -> &str {
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
