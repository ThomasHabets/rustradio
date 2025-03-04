/*! Vector to stream block.

Turn stream of e.g. `Vec<u8>` to stream of `u8`.
 */
use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, WriteStream};

/// Block for vector to stream.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct VecToStream<T> {
    #[rustradio(in)]
    src: NCReadStream<Vec<T>>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Copy> Block for VecToStream<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let n = match self.src.peek_size() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some(x) => x,
        };
        let mut o = self.dst.write_buf()?;
        if n > o.len() {
            return Ok(BlockRet::Ok);
        }
        let (v, _tags) = self
            .src
            .pop()
            .expect("we just checked the size. It must exist");
        // TODO: write start and end tags.
        o.fill_from_iter(v);
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}
