/*! Vector to stream block.

Turn stream of e.g. `Vec<u8>` to stream of `u8`.
 */
use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
use crate::Error;

/// Block for vector to stream.
pub struct VecToStream<T> {
    src: Streamp<Vec<T>>,
    dst: Streamp<T>,
}

impl<T> VecToStream<T> {
    /// Create new VecToStream.
    pub fn new(src: Streamp<Vec<T>>) -> Self {
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
        let mut i = self.src.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.lock()?;
        for v in i.iter() {
            // TODO: write start and end tags.
            o.write_slice(v);
        }
        i.clear();
        Ok(BlockRet::Ok)
    }
}
