/*! Vector to stream block.

Turn stream of e.g. `Vec<u8>` to stream of `u8`.
 */
use log::trace;

use crate::Error;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, Tag, TagValue, WriteStream};

/// This tag gets added to the first output sample of a vec.
///
/// The value is the number of samples in the vector.
pub const TAG_START: &str = "VecToStream::start";

/// This tag gets added to the last output sample of a vec.
///
/// The value is the number of samples in the vector.
pub const TAG_END: &str = "VecToStream::end";

/// Block for vector to stream.
///
/// The output stream is tagged with `VecToStream::start` and `VecToStream::end`
/// on the first and last sample of the stream. For a one-sample vector, these
/// tags will be on the same sample.
///
/// Empty vectors are silently discarded.
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
            return Ok(BlockRet::WaitForStream(&self.src, n));
        }
        let (v, mut tags) = self
            .src
            .pop()
            .expect("we just checked the size. It must exist");
        debug_assert_eq!(v.len(), n);
        if n == 0 {
            trace!("VecToStream: discarded empty vector");
            return Ok(BlockRet::Ok);
        }
        o.fill_from_iter(v);
        tags.extend([
            Tag::new(0, TAG_START.to_string(), TagValue::U64(n as u64)),
            Tag::new(n - 1, TAG_END.to_string(), TagValue::U64(n as u64)),
        ]);
        o.produce(n, &tags);
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_nocopy_stream;
    use anyhow::Result;

    #[test]
    fn empty_input() -> Result<()> {
        let (_tx, rx) = new_nocopy_stream();
        let (mut b, out) = VecToStream::<u8>::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.read_buf()?.0.len(), 0);
        Ok(())
    }

    #[test]
    fn empty_vec() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(vec![], &[]);
        let (mut b, out) = VecToStream::<u8>::new(rx);
        assert!(matches![b.work()?, BlockRet::Ok]);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.read_buf()?.0.len(), 0);
        Ok(())
    }

    #[test]
    fn two() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(vec![11, 22, 33], &[]);
        tx.push(vec![3, 2, 1, 0], &[]);
        let (mut b, out) = VecToStream::<u8>::new(rx);
        assert!(matches![b.work()?, BlockRet::Ok]);
        assert_eq!(out.read_buf()?.0.len(), 3);
        assert!(matches![b.work()?, BlockRet::Ok]);
        assert_eq!(out.read_buf()?.0.len(), 7);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (o, tags) = out.read_buf()?;
        assert_eq!(o.len(), 7);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "VecToStream::start".to_string(), TagValue::U64(3)),
                Tag::new(2, "VecToStream::end".to_string(), TagValue::U64(3)),
                Tag::new(3, "VecToStream::start".to_string(), TagValue::U64(4)),
                Tag::new(6, "VecToStream::end".to_string(), TagValue::U64(4)),
            ]
        );
        Ok(())
    }
}
