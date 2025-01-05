//! Xor two streams.
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::Error;

/// Xors a constant value to every sample.
// TODO: make this sync
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out)]
pub struct Xor<T>
where
    T: Copy,
{
    #[rustradio(in)]
    a: ReadStream<T>,
    #[rustradio(in)]
    b: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> Block for Xor<T>
where
    T: Copy + std::ops::BitXor<Output = T>,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (a, tags) = self.a.read_buf()?;
        let (b, _tags) = self.b.read_buf()?;
        let n = std::cmp::min(a.len(), b.len());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        let n = std::cmp::min(n, o.len());
        let it = a.iter().zip(b.iter()).map(|(x, y)| *x ^ *y);
        for (w, samp) in o.slice().iter_mut().take(n).zip(it) {
            *w = samp;
        }
        a.consume(n);
        b.consume(n);
        o.produce(n, &tags);
        Ok(BlockRet::Ok)
    }
}
