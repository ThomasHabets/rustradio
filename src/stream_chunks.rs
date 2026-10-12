//! Stream to PDU with fixed size.
use crate::block::{Block, BlockRet};
use crate::stream::{NCWriteStream, ReadStream};
use crate::{Error, Result, Sample};

/// Stream to PDU block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct StreamChunks<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<T>>,
    size: usize,
}

impl<T: Sample> StreamChunks<T> {
    /// Create fixed-size PDUs, carrying tags covered by each chunk.
    ///
    /// Returns an error for zero size or a size exceeding input capacity.
    pub fn new(
        src: ReadStream<T>,
        size: usize,
    ) -> Result<(Self, crate::stream::NCReadStream<Vec<T>>)> {
        if size == 0 || size > src.total_size() {
            return Err(Error::msg(
                "StreamChunks size must be nonzero and fit input capacity",
            ));
        }
        let (dst, out) = crate::stream::new_nocopy_stream();
        Ok((Self { src, dst, size }, out))
    }
}

impl<T: Sample> Block for StreamChunks<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            let output_space = self.dst.remaining();
            if output_space == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let (input, intags) = self.src.read_buf()?;
            {
                let inlen = input.len();
                if inlen < self.size {
                    return Ok(BlockRet::WaitForStream(&self.src, self.size));
                }
            }
            self.dst.push(
                input.slice()[..self.size].to_vec(),
                intags
                    .into_iter()
                    .take_while(|t| t.pos() < self.size)
                    .collect::<Vec<_>>(),
            );
            input.consume(self.size);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_zero_chunk_size() {
        assert!(StreamChunks::<u8>::new(ReadStream::from_slice(&[]), 0).is_err());
    }

    use super::*;
    use crate::Complex;
    use crate::blocks::VectorSource;

    #[test]
    fn tags_at_chunk_boundaries_stay_with_their_samples() -> Result<()> {
        use crate::stream::{Tag, TagValue, new_stream};

        let (writer, reader) = new_stream()?;
        let (mut block, out) = StreamChunks::new(reader, 4)?;
        let tag = |pos, key| Tag::new(pos, key, TagValue::Bool(true));
        let mut input = writer.write_buf()?;
        input.fill_from_slice(&[0u8, 1, 2, 3, 4, 5, 6, 7]);
        // Writers may supply unordered tags. Readers sort them stably before
        // the block selects the prefix belonging to each chunk.
        input.produce(
            8,
            &[
                tag(7, "last"),
                tag(4, "boundary"),
                tag(1, "a"),
                tag(1, "b"),
                tag(0, "first"),
            ],
        );
        block.work()?;
        let (data, tags) = out.pop().unwrap();
        assert_eq!(data, [0, 1, 2, 3]);
        assert_eq!(tags, [tag(0, "first"), tag(1, "a"), tag(1, "b")]);
        let (data, tags) = out.pop().unwrap();
        assert_eq!(data, [4, 5, 6, 7]);
        assert_eq!(tags, [tag(0, "boundary"), tag(3, "last")]);
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn rejects_chunks_larger_than_input_capacity() {
        let (_tx, rx) = crate::stream::new_stream::<u32>().unwrap();
        let size = rx.total_size() + 1;
        assert!(StreamChunks::new(rx, size).is_err());
    }

    #[test]
    fn even() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![Complex::default(); 70]).build()?;
        let (mut b, out) = StreamChunks::new(src_out, 10)?;
        assert!(matches![src.work()?, BlockRet::EOF]);
        let r = b.work()?;
        assert!(matches![r, BlockRet::WaitForStream(_, 10)], "Was {r:?}");
        let mut n = 0;
        while let Some((v, tags)) = out.pop() {
            assert_eq!(v.len(), 10);
            if n != 0 {
                assert_eq!(tags.len(), 0);
            }
            n += 1;
        }
        assert_eq!(n, 7);
        Ok(())
    }

    #[test]
    fn not_even() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![Complex::default(); 72]).build()?;
        let (mut b, out) = StreamChunks::new(src_out, 10)?;
        assert!(matches![src.work()?, BlockRet::EOF]);
        let r = b.work()?;
        assert!(matches![r, BlockRet::WaitForStream(_, 10)], "Was {r:?}");
        let mut n = 0;
        while let Some((v, tags)) = out.pop() {
            assert_eq!(v.len(), 10);
            if n != 0 {
                assert_eq!(tags.len(), 0);
            }
            n += 1;
        }
        assert_eq!(n, 7);
        Ok(())
    }
}
