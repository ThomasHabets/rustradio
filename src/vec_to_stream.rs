/*! Vector to stream block.

Turn stream of e.g. `Vec<u8>` to stream of `u8`.
 */
use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, NoCopyStreamp, Streamp};
use crate::Error;

/// Block for vector to stream.
pub struct VecToStream<T> {
    src: NoCopyStreamp<Vec<T>>,
    dst: Streamp<T>,
}

impl<T> VecToStream<T> {
    /// Create new VecToStream.
    pub fn new(src: NoCopyStreamp<Vec<T>>) -> Self {
        Self {
            src,
            dst: new_streamp(),
        }
    }
    /// Return output stream.
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T: Copy> Block for VecToStream<T> {
    fn block_name(&self) -> &'static str {
        "VecToStream"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let n = match self.src.peek_size() {
            None => return Ok(BlockRet::Noop),
            Some(x) => x,
        };
        let mut o = self.dst.write_buf()?;
        if n > o.len() {
            return Ok(BlockRet::Ok);
        }
        let v = self
            .src
            .pop()
            .expect("we just checked the size. It must exist");
        // TODO: write start and end tags.
        o.fill_from_iter(v);
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}
