//! Stream to PDU.
use std::collections::HashMap;

use log::{debug, trace};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagPos, TagValue};
use crate::{Result, Sample};

/// Stream to PDU block.
///
/// Turn a tagged stream to PDUs.
///
/// PDUs are marked in the stream as `true` when they start, and `false` when
/// they end. Optionally an extra `tail` samples are also included.
///
/// The sample with the `false` tag is not included, unless `tail` is greater
/// than zero.
///
/// Samples between bursts are discarded.
///
/// ## Example
///
/// This example uses burst tagger to create the tags, and turn a stream
/// into burst PDUs.
///
/// Also see `examples/wpcr.rs`.
///
/// ```
/// use rustradio::graph::{Graph, GraphRunner};
/// use rustradio::blocks::{FileSource, Tee, ComplexToMag2, SinglePoleIirFilter,BurstTagger,StreamToPdu};
/// use rustradio::Complex;
/// let (src, src_out) = FileSource::new("/dev/null")?;
/// let (tee, data, b) = Tee::new(src_out);
/// let (c2m, c2m_out) = ComplexToMag2::new(b);
/// let (iir, iir_out) = SinglePoleIirFilter::new(c2m_out, 0.01).unwrap();
/// let (burst, prev) = BurstTagger::new(data, iir_out, 0.0001, "burst");
/// let pdus = StreamToPdu::new(prev, "burst", 10_000, 50);
/// // pdus.out() now delivers bursts as Vec<Complex>
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct StreamToPdu<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<T>>,
    tag: String,
    buf: Vec<T>,
    endcounter: Option<usize>,
    max_size: usize,
    tail: usize,
}

impl<T: Sample> StreamToPdu<T> {
    /// Make new Stream to PDU block.
    pub fn new<S: Into<String>>(
        src: ReadStream<T>,
        tag: S,
        max_size: usize,
        tail: usize,
    ) -> (Self, NCReadStream<Vec<T>>) {
        let (dst, dr) = crate::stream::new_nocopy_stream();
        (
            Self {
                src,
                tag: tag.into(),
                dst,
                buf: Vec::with_capacity(max_size),
                endcounter: None,
                max_size,
                tail,
            },
            dr,
        )
    }

    /// Burst has arrived. File it.
    fn done(&mut self) {
        let mut delme = Vec::with_capacity(self.max_size);
        std::mem::swap(&mut delme, &mut self.buf);
        debug!(
            "StreamToPdu> got burst of size {} samples, {} bytes",
            delme.len(),
            delme.len() * T::size()
        );
        // TODO: record stream pos.
        self.dst.push(delme, &[]);
        self.endcounter = None;
    }
}

// If a given tag exists at the given position, return Some(that bool). Else
// return None.
fn get_tag_val_bool(tags: &HashMap<(TagPos, &str), &Tag>, pos: TagPos, key: &str) -> Option<bool> {
    if let Some(tag) = tags.get(&(pos, key)) {
        match tag.val() {
            TagValue::Bool(b) => Some(*b),
            _ => None,
        }
    } else {
        None
    }
}

impl<T: Sample> Block for StreamToPdu<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (input, tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }

        // TODO: we actually only care about one single tag,
        // and I think we should drop the rest no matter what.
        let tags = tags
            .iter()
            .map(|t| ((t.pos(), t.key()), t))
            .collect::<HashMap<(TagPos, &str), &Tag>>();
        trace!("StreamToPdu: tags: {tags:?}");

        for (i, sample) in input.iter().enumerate() {
            //eprintln!("sample: {i} {sample:?}, {:?}", self.endcounter);
            if let Some(c) = self.endcounter {
                self.buf.push(*sample);
                self.endcounter = Some(c - 1);
                if c == 1 {
                    self.done();
                }
            } else if let Some(tv) = get_tag_val_bool(&tags, i as TagPos, &self.tag) {
                if !tv {
                    // End of burst.
                    if self.tail > 0 {
                        self.buf.push(*sample);
                    }
                    if self.tail <= 1 {
                        self.done();
                    } else {
                        self.endcounter = Some(self.tail - 1);
                    }
                } else {
                    // Start of burst, save first sample.
                    self.buf.push(*sample);
                }
            } else if !self.buf.is_empty() {
                // Burst continuation.
                self.buf.push(*sample);
            }
            if self.buf.len() > self.max_size {
                // Too long. Discard buffer and stop saving.
                self.buf.clear();
                self.endcounter = None;
            }
        }
        let n = input.len();
        input.consume(n);
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;
    use crate::blocks::VectorSource;

    #[test]
    fn no_pdu() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![Complex::default(); 100]).build()?;
        let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, 0);
        assert!(matches![src.work()?, BlockRet::EOF]);
        assert!(matches![b.work()?, BlockRet::Again]);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn single() -> Result<()> {
        for (start, end, tail, want) in [
            (0, 7, 0, vec![1, 2, 3, 4, 5, 6, 7]),
            (0, 0, 0, vec![]),
            (0, 0, 1, vec![1]),
            (1, 1, 0, vec![]),
            (1, 1, 1, vec![2]),
            (1, 1, 9, vec![2, 3, 4, 5, 6, 7, 8, 9, 10]),
            (7, 7, 0, vec![]),
            (7, 7, 1, vec![8]),
            (7, 7, 2, vec![8, 9]),
            (7, 7, 3, vec![8, 9, 10]),
            (7, 8, 0, vec![8]),
            (7, 8, 1, vec![8, 9]),
            (7, 8, 2, vec![8, 9, 10]),
            (7, 9, 0, vec![8, 9]),
            (7, 9, 1, vec![8, 9, 10]),
        ] {
            eprintln!("Testing with end={end}, tail={tail}, want={want:?}");
            let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .tags(&[
                    Tag::new(start, "burst", TagValue::Bool(true)),
                    Tag::new(4, "test", TagValue::Bool(true)),
                    Tag::new(end, "burst", TagValue::Bool(false)),
                ])
                .build()?;
            let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, tail);
            assert!(matches![src.work()?, BlockRet::EOF]);
            assert!(matches![b.work()?, BlockRet::Again]);
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
            let (burst, tags) = out.pop().unwrap();
            assert_eq!(burst, want);
            assert_eq!(tags, &[]);
            assert!(out.pop().is_none());
        }
        Ok(())
    }

    #[test]
    fn ended_too_soon() -> Result<()> {
        for (end, tail) in [(7, 4), (8, 3), (9, 2)] {
            eprintln!("Testing with end={end}, tail={tail}");
            let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .tags(&[
                    Tag::new(7, "burst", TagValue::Bool(true)),
                    Tag::new(4, "test", TagValue::Bool(true)),
                    Tag::new(end, "burst", TagValue::Bool(false)),
                ])
                .build()?;
            let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, tail);
            assert!(matches![src.work()?, BlockRet::EOF]);
            assert!(matches![b.work()?, BlockRet::Again]);
            assert!(out.pop().is_none());
        }
        Ok(())
    }

    #[test]
    fn mid_pdu() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
            .tags(&[
                Tag::new(3, "burst", TagValue::Bool(true)),
                Tag::new(4, "test", TagValue::Bool(true)),
                Tag::new(7, "burst", TagValue::Bool(false)),
            ])
            .build()?;
        let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, 0);
        assert!(matches![src.work()?, BlockRet::EOF]);
        assert!(matches![b.work()?, BlockRet::Again]);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (burst, tags) = out.pop().unwrap();
        assert_eq!(burst, &[4, 5, 6, 7]);
        assert_eq!(tags, &[]);
        assert!(out.pop().is_none());
        Ok(())
    }
}
