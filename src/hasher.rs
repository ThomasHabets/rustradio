//! Hash input until EOF, outputting the results.
use anyhow::Result;
use sha2::Digest;

use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream};

#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct Hasher {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,

    hasher: sha2::Sha512,
}

impl Hasher {
    /// Create a new block.
    ///
    /// The arguments to this function are the mandatory input
    /// streams, and the mandatory parameters.
    ///
    /// The return values are the block itself, plus any mandatory
    /// output streams.
    ///
    /// This function is automatically generated by a macro.
    // TODO: fix it so that the macro can generate this.
    pub fn new(
        src: ReadStream<u8>,
        hasher: sha2::Sha512,
    ) -> (Self, crate::stream::NCReadStream<Vec<u8>>) {
        let dst = crate::stream::new_nocopy_stream();
        (
            Self {
                src,
                dst: dst.0,
                hasher,
            },
            dst.1,
        )
    }
}

impl Block for Hasher {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _) = self.src.read_buf()?;
        let n = i.len();
        self.hasher.update(i.slice());
        i.consume(n);
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}

impl Drop for Hasher {
    fn drop(&mut self) {
        let res = self.hasher.clone().finalize();
        self.dst.push(res.to_vec(), &[]);
    }
}

pub fn sha512(src: ReadStream<u8>) -> (Hasher, NCReadStream<Vec<u8>>) {
    Hasher::new(src, sha2::Sha512::new())
}
