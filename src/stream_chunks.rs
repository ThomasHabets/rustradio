//! Stream to PDU with fixed size.
use crate::block::{Block, BlockRet};
use crate::stream::{NCWriteStream, ReadStream};
use crate::{Result, Sample};

/// Stream to PDU block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct StreamChunks<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<T>>,
    size: usize,
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
                    return Ok(BlockRet::WaitForStream(&self.src, self.size - inlen));
                }
            }
            self.dst.push(
                input.slice()[..self.size].to_vec(),
                intags
                    .into_iter()
                    .filter(|t| t.pos() < self.size)
                    .collect::<Vec<_>>(),
            );
            input.consume(self.size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;
    use crate::blocks::VectorSource;

    #[test]
    fn even() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![Complex::default(); 70]).build()?;
        let (mut b, out) = StreamChunks::new(src_out, 10);
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
        let (mut b, out) = StreamChunks::new(src_out, 10);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let r = b.work()?;
        assert!(matches![r, BlockRet::WaitForStream(_, 8)], "Was {r:?}");
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
