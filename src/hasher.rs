//! Hash input until EOF, outputting the results.
use sha2::Digest;

use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream};

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Hasher {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,

    hasher: sha2::Sha512,
}

impl Block for Hasher {
    fn work(&mut self) -> Result<BlockRet> {
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

#[must_use]
pub fn sha512(src: ReadStream<u8>) -> (Hasher, NCReadStream<Vec<u8>>) {
    Hasher::new(src, sha2::Sha512::new())
}
