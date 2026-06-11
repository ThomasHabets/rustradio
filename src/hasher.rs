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

    #[rustradio(default)]
    done: bool,
}

impl Block for Hasher {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        if self.src.eof() {
            // This will not necessarily be called. We normally return
            // `WaitForStream`, so graph may not call us again.
            //
            // If we don't get called again, though, we cound on the Graph to
            // drop us, and we'll finish up in the destructor.
            self.finish();
            return Ok(BlockRet::EOF);
        }
        let (i, _) = self.src.read_buf()?;
        let n = i.len();
        self.hasher.update(i.slice());
        i.consume(n);
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}

impl Hasher {
    fn finish(&mut self) {
        if self.done {
            return;
        }
        let res = self.hasher.clone().finalize();
        self.dst.push(res.to_vec(), &[]);
        self.done = true;
    }
}

impl Drop for Hasher {
    fn drop(&mut self) {
        self.finish();
    }
}

#[must_use]
pub fn sha512(src: ReadStream<u8>) -> (Hasher, NCReadStream<Vec<u8>>) {
    Hasher::new(src, sha2::Sha512::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockRet};

    #[test]
    fn emits_hash_when_input_closes() -> Result<()> {
        let (tx, rx) = crate::stream::new_stream();
        {
            let mut w = tx.write_buf()?;
            w.fill_from_slice(b"abc");
            w.produce(3, &[]);
        }
        drop(tx);

        let (mut hasher, out) = sha512(rx);
        assert!(matches![hasher.work()?, BlockRet::WaitForStream(_, 1)]);
        assert!(matches![hasher.work()?, BlockRet::EOF]);

        let (got, _) = out.pop().expect("hasher should emit a digest");
        assert_eq!(got, sha2::Sha512::digest(b"abc").to_vec());
        Ok(())
    }

    #[test]
    fn emits_hash_when_dropped() -> Result<()> {
        let (tx, rx) = crate::stream::new_stream();
        {
            let mut w = tx.write_buf()?;
            w.fill_from_slice(b"abc");
            w.produce(3, &[]);
        }
        drop(tx);

        let (mut hasher, out) = sha512(rx);
        assert!(matches![hasher.work()?, BlockRet::WaitForStream(_, 1)]);
        drop(hasher);

        let (got, _) = out.pop().expect("hasher should emit a digest");
        assert_eq!(got, sha2::Sha512::digest(b"abc").to_vec());
        Ok(())
    }
}
