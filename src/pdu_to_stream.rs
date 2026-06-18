//! PDU to stream.

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, Tag, TagValue, WriteStream};
use crate::{Result, Sample};

/// This tag gets added to the first output sample of a vec.
///
/// The value is the number of samples in the vector.
pub const TAG_START: &str = "PduToStream::start";

/// This tag gets added to the last output sample of a vec.
///
/// The value is the number of samples in the vector.
pub const TAG_END: &str = "PduToStream::end";

/// PDU to stream block.
///
/// The output stream is tagged with `PduToStream::start` and `PduToStream::end`
/// on the first and last sample of the stream. For a one-sample vector, these
/// tags will be on the same sample.
///
/// Empty PDUs are silently discarded.
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
    #[rustradio(default)]
    pdu_len: u64,
}

impl<T: Sample> Block for PduToStream<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }

            while self.buf.is_empty() {
                let Some((pdu, tags)) = self.src.pop() else {
                    return Ok(BlockRet::WaitForStream(&self.src, 1));
                };
                if pdu.is_empty() {
                    continue;
                }
                self.pdu_len = pdu.len() as u64;
                self.buf = pdu;
                self.tags = tags;
                self.tags
                    .push(Tag::new(0, TAG_START, TagValue::U64(self.pdu_len)));
            }
            let n = std::cmp::min(o.len(), self.buf.len());
            assert_ne!(n, 0, "we already checked");
            o.slice()[..n].copy_from_slice(&self.buf[..n]);
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
            self.buf.drain(..n);
            if self.buf.is_empty() {
                tags.push(Tag::new(n - 1, TAG_END, TagValue::U64(self.pdu_len)));
            }
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
            &[
                Tag::new(123, "one-two-three", TagValue::Bool(false)),
                Tag::new(len - 1, "tail", TagValue::Bool(true)),
            ],
        );
        let (mut b, out) = PduToStream::new(rx);

        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (buf, tags) = out.read_buf()?;
        assert_eq!(buf.len(), DEFAULT_STREAM_SIZE);
        assert_eq!(
            tags,
            vec![
                Tag::new(
                    0,
                    "PduToStream::start",
                    TagValue::U64((DEFAULT_STREAM_SIZE + 1) as u64)
                ),
                Tag::new(123, "one-two-three", TagValue::Bool(false)),
            ]
        );
        buf.consume(DEFAULT_STREAM_SIZE);

        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (buf, tags) = out.read_buf()?;
        assert_eq!(buf.len(), 1);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "tail", TagValue::Bool(true)),
                Tag::new(
                    0,
                    "PduToStream::end",
                    TagValue::U64((DEFAULT_STREAM_SIZE + 1) as u64)
                )
            ]
        );
        Ok(())
    }

    #[test]
    fn empty_input() -> Result<()> {
        let (_tx, rx) = new_nocopy_stream();
        let (mut b, out) = PduToStream::<u8>::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.read_buf()?.0.len(), 0);
        Ok(())
    }

    #[test]
    fn empty_vec() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(vec![], &[]);
        let (mut b, out) = PduToStream::<u8>::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.read_buf()?.0.len(), 0);
        Ok(())
    }

    #[test]
    fn two() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(vec![11, 22, 33], &[]);
        tx.push(vec![3, 2, 1, 0], &[]);
        let (mut b, out) = PduToStream::<u8>::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.read_buf()?.0.len(), 7);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (o, tags) = out.read_buf()?;
        assert_eq!(o.len(), 7);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "PduToStream::start", TagValue::U64(3)),
                Tag::new(2, "PduToStream::end", TagValue::U64(3)),
                Tag::new(3, "PduToStream::start", TagValue::U64(4)),
                Tag::new(6, "PduToStream::end", TagValue::U64(4)),
            ]
        );
        Ok(())
    }
}
