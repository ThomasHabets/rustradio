//! PDU to stream.

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, Tag, WriteStream};
use crate::{Result, Sample};

/// PDU to stream block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct PduToStream<T: Sample> {
    #[rustradio(in)]
    src: NCReadStream<Vec<T>>,
    #[rustradio(out)]
    dst: WriteStream<T>,

    // Buf contains at most one packet. We use a local buffer instead of peeking
    // into the size of what we're about to pop because the packet could be
    // bigger than the maximum output buffer.
    #[rustradio(default)]
    buf: Vec<T>,
    #[rustradio(default)]
    tags: Vec<Tag>,
}

impl<T: Sample> Block for PduToStream<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            while self.buf.is_empty() {
                let Some((pdu, tags)) = self.src.pop() else {
                    return Ok(BlockRet::WaitForStream(&self.src, 1));
                };
                self.buf = pdu;
                // TODO: add some tags of our own.
                self.tags = tags;
            }
            let mut o = self.dst.write_buf()?;
            let n = std::cmp::min(o.len(), self.buf.len());
            if n == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            o.slice()[..n].copy_from_slice(&self.buf[..n]);
            self.buf.drain(0..n);
            let mut tags = Vec::new();
            self.tags.retain_mut(|tag| {
                if tag.pos() < n {
                    tags.push(tag.clone());
                    false
                } else {
                    tag.set_pos(tag.pos() - n);
                    true
                }
            });
            o.produce(n, &tags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::stream::{DEFAULT_STREAM_SIZE, TagValue, new_nocopy_stream};

    #[test]
    fn carries_tags_across_partial_output_writes() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let len = DEFAULT_STREAM_SIZE + 1;
        tx.push(
            vec![0u8; len],
            &[Tag::new(len - 1, "tail", TagValue::Bool(true))],
        );
        let (mut b, out) = PduToStream::new(rx);

        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (buf, tags) = out.read_buf()?;
        assert_eq!(buf.len(), DEFAULT_STREAM_SIZE);
        assert!(tags.is_empty());
        buf.consume(DEFAULT_STREAM_SIZE);

        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (buf, tags) = out.read_buf()?;
        assert_eq!(buf.len(), 1);
        assert_eq!(tags, &[Tag::new(0, "tail", TagValue::Bool(true))]);
        Ok(())
    }
}
