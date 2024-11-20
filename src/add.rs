//! Add two streams.
use crate::block::{AutoBlock, Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

/// Adds two streams, sample wise.
#[derive(rustradio_macros::Block)]
#[rustradio(new, out)]
pub struct Add<T>
where
    T: Copy,
{
    //t: u32,
    /// Hello world.
    #[rustradio(in)]
    a: Streamp<T>,

    #[rustradio(in)]
    b: Streamp<T>,

    #[rustradio(out)]
    dst: Streamp<T>,
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
