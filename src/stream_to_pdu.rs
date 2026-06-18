//! Stream to PDU.
use std::collections::HashMap;

use log::{debug, trace};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagPos, TagValue};
use crate::{Result, Sample};

#[derive(Default)]
enum State<T: Sample> {
    #[default]
    Unsync,
    Packet(Vec<T>, Vec<Tag>),
    Tail(Vec<T>, Vec<Tag>, usize),
}

impl<T: Sample> State<T> {
    fn len(&self) -> usize {
        match self {
            State::Unsync => 0,
            State::Packet(p, _) => p.len(),
            State::Tail(p, _, _) => p.len(),
        }
    }
}

impl<T: Sample> std::fmt::Debug for State<T> {
    fn fmt(&self, w: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            State::Unsync => write!(w, "Unsync"),
            State::Packet(p, tags) => write!(w, "Packet len={} tags={}", p.len(), tags.len()),
            State::Tail(p, tags, tail) => {
                write!(w, "Tail len={} tail={tail} tags={}", p.len(), tags.len())
            }
        }
    }
}

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
    state: State<T>,

    // Count how many samples are left of the tail.
    // `None` means that we are not currently inside the tail.
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
                state: State::Unsync,
                max_size,
                tail,
            },
            dr,
        )
    }

    /// Burst has arrived. File it.
    fn file_burst(&mut self, v: impl Into<Vec<T>>, tags: Vec<Tag>) {
        let v = v.into();
        if v.len() > self.max_size {
            return;
        }
        debug!(
            "StreamToPdu> got burst of size {} samples, {} bytes",
            v.len(),
            v.len() * T::size()
        );
        // TODO: record stream pos.
        self.dst.push(v, tags);
        self.state = State::Unsync;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BurstTag {
    None,
    Start,
    End,
    Both,
}

// If a given tag exists at the given position, return Some(that bool). Else
// return None.
fn get_tag_val_bool(tags: &HashMap<(TagPos, &str), Vec<&Tag>>, pos: TagPos, key: &str) -> BurstTag {
    let mut i = 0;
    if let Some(ts) = tags.get(&(pos, key)) {
        for tag in ts {
            match tag.val() {
                TagValue::Bool(true) => i |= 1,
                TagValue::Bool(false) => i |= 2,
                _ => {} // ignore non-bool tag.
            }
        }
    }
    match i {
        0 => BurstTag::None,
        1 => BurstTag::Start,
        2 => BurstTag::End,
        3 => BurstTag::Both,
        other => panic!("impossible value {other}"),
    }
}

fn tags_pos_adjust(pos: usize, intags: Option<&Vec<Tag>>) -> Vec<Tag> {
    intags
        .map(|v| {
            v.iter()
                .map(|e| Tag::new(pos, e.key(), e.val().clone()))
                .collect()
        })
        .unwrap_or_default()
}

impl<T: Sample> Block for StreamToPdu<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let output_space = self.dst.remaining();
        if output_space == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let (input, intags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }

        // TODO: we actually only care about one single tag,
        // and I think we should drop the rest no matter what.
        let tags = {
            let mut tags: HashMap<(usize, &str), Vec<&Tag>> = HashMap::new();
            for e in &intags {
                tags.entry((e.pos(), e.key())).or_default().push(e);
            }
            tags
        };
        let other_tags = {
            let mut tags: HashMap<usize, Vec<Tag>> = HashMap::new();
            for e in &intags {
                if e.key() != self.tag {
                    tags.entry(e.pos()).or_default().push(e.clone());
                }
            }
            tags
        };
        trace!("StreamToPdu: tags: {tags:?}");

        for (i, sample) in input.iter().enumerate() {
            let tagvalue = get_tag_val_bool(&tags, i as TagPos, &self.tag);

            eprintln!("State: {:?} & {tagvalue:?}", self.state);
            self.state = match (&mut self.state, tagvalue) {
                (State::Unsync, BurstTag::None | BurstTag::End) => State::Unsync,
                (State::Unsync, BurstTag::Start) => State::Packet(
                    vec![*sample],
                    tags_pos_adjust(0, other_tags.get(&(i as TagPos))),
                ),
                (State::Unsync, BurstTag::Both) => {
                    if self.tail > 0 {
                        State::Tail(
                            vec![*sample],
                            tags_pos_adjust(0, other_tags.get(&(i as TagPos))),
                            self.tail - 1,
                        )
                    } else {
                        // TODO: is this needed?
                        self.file_burst(vec![], vec![]);
                        State::Unsync
                    }
                }
                (State::Packet(p, tags), BurstTag::Start | BurstTag::None) => {
                    // Should we reset the burst on Start? Make sure it's consistent with
                    // Packet/Both and Tail/Start/Both.
                    let mut p = std::mem::take(p);
                    let mut tags = std::mem::take(tags);
                    tags.extend(tags_pos_adjust(p.len(), other_tags.get(&(i as TagPos))));
                    p.push(*sample);
                    State::Packet(p, tags)
                }

                // Should we reset the burst on `Both`? Make sure it's consistent with
                // Packet/Start and Tail/Start/Both.
                (State::Packet(p, tags), BurstTag::Both | BurstTag::End) => {
                    let mut tail = self.tail;
                    let mut p = std::mem::take(p);
                    let mut tags = std::mem::take(tags);
                    if tail > 0 {
                        tags.extend(tags_pos_adjust(p.len(), other_tags.get(&(i as TagPos))));
                        p.push(*sample);
                        tail -= 1;
                    }
                    if tail > 0 {
                        State::Tail(p, tags, tail)
                    } else {
                        self.file_burst(p, tags);
                        State::Unsync
                    }
                }

                // Ignore burst tags while in tail. (see sync with above)
                (State::Tail(p, tags, tail), _) => {
                    //let mut p = std::mem::take(p);
                    if *tail > 0 {
                        tags.extend(tags_pos_adjust(p.len(), other_tags.get(&(i as TagPos))));
                        p.push(*sample);
                        *tail -= 1;
                    }
                    if *tail == 0 {
                        let p = std::mem::take(p);
                        let tags = std::mem::take(tags);
                        self.file_burst(p, tags);
                        State::Unsync
                    } else {
                        State::Tail(std::mem::take(p), std::mem::take(tags), *tail)
                    }
                }
            };
            if self.state.len() > self.max_size {
                self.state = State::Unsync;
            }
        }
        let n = input.len();
        input.consume(n);
        Ok(BlockRet::WaitForStream(&self.src, 1))
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
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn single() -> Result<()> {
        for (start, end, tail, want) in [
            (0, 7, 0, vec![vec![1, 2, 3, 4, 5, 6, 7]]),
            (0, 0, 0, vec![vec![]]),
            (0, 1, 0, vec![vec![1]]),
            (0, 0, 1, vec![vec![1]]),
            (1, 1, 0, vec![vec![]]),
            (1, 1, 1, vec![vec![2]]),
            (1, 1, 3, vec![vec![2, 3, 4]]),
            (1, 1, 9, vec![vec![2, 3, 4, 5, 6, 7, 8, 9, 10]]),
            (9, 7, 0, vec![]),
            (7, 7, 1, vec![vec![8]]),
            (7, 7, 2, vec![vec![8, 9]]),
            (7, 7, 3, vec![vec![8, 9, 10]]),
            (7, 8, 0, vec![vec![8]]),
            (7, 8, 1, vec![vec![8, 9]]),
            (7, 8, 2, vec![vec![8, 9, 10]]),
            (7, 9, 0, vec![vec![8, 9]]),
            (7, 9, 1, vec![vec![8, 9, 10]]),
        ] {
            eprintln!("Testing with start={start} end={end}, tail={tail}, want={want:?}");
            let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .tags(&[
                    Tag::new(start, "burst", TagValue::Bool(true)),
                    Tag::new(4, "test", TagValue::Bool(true)),
                    Tag::new(end, "burst", TagValue::Bool(false)),
                ])
                .build()?;
            let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, tail);
            assert!(matches![src.work()?, BlockRet::EOF]);
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
            for w in want.into_iter() {
                let (burst, tags) = out.pop().unwrap();
                assert_eq!(burst, w);
                let mut want_tags: Vec<Tag> = Vec::new();
                if start == 0 && !burst.is_empty() {
                    want_tags.extend([
                        Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                        Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                        Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                    ]);
                }
                if start <= 4 && (end + tail) > 4 {
                    want_tags.push(Tag::new(4 - start, "test", TagValue::Bool(true)));
                }
                assert_eq!(tags, want_tags);
            }
            assert_eq!(out.pop(), None);
        }
        Ok(())
    }

    #[test]
    fn size() -> Result<()> {
        for (start, end, tail, want) in [
            // Start.
            (0, 0, 0, vec![vec![]]),
            (0, 1, 0, vec![vec![1u8]]),
            (0, 2, 0, vec![vec![1u8, 2]]),
            (0, 3, 0, vec![vec![1u8, 2, 3]]),
            (0, 4, 0, vec![]),
            (0, 5, 0, vec![]),
            // Mid.
            (1, 1, 0, vec![vec![]]),
            (1, 2, 0, vec![vec![2u8]]),
            (1, 3, 0, vec![vec![2u8, 3]]),
            (1, 4, 0, vec![vec![2u8, 3, 4]]),
            (1, 5, 0, vec![]),
            (1, 6, 0, vec![]),
            // Tail.
            (0, 0, 1, vec![vec![1]]),
            (0, 1, 1, vec![vec![1, 2]]),
            (0, 2, 1, vec![vec![1, 2, 3]]),
            (0, 3, 1, vec![]),
            (0, 4, 1, vec![]),
            // Tail + mid.
            (1, 1, 1, vec![vec![2]]),
            (1, 2, 1, vec![vec![2, 3]]),
            (1, 3, 1, vec![vec![2, 3, 4]]),
            (1, 4, 1, vec![]),
            (1, 5, 1, vec![]),
        ] {
            eprintln!("Testing with start={start} end={end}, tail={tail}, want={want:?}");
            let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .tags(&[
                    Tag::new(start, "burst", TagValue::Bool(true)),
                    Tag::new(4, "test", TagValue::Bool(true)),
                    Tag::new(end, "burst", TagValue::Bool(false)),
                ])
                .build()?;
            let (mut b, out) = StreamToPdu::new(src_out, "burst", 3, tail);
            assert!(matches![src.work()?, BlockRet::EOF]);
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
            for w in want.into_iter() {
                let (burst, tags) = out.pop().unwrap();
                assert_eq!(burst, w);
                let mut want_tags: Vec<Tag> = Vec::new();
                if start == 0 && !burst.is_empty() {
                    want_tags.extend([
                        Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                        Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                        Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                    ]);
                }
                if start <= 4 && (end + tail) > 4 {
                    want_tags.push(Tag::new(4 - start, "test", TagValue::Bool(true)));
                }
                assert_eq!(tags, tags);
            }
            assert_eq!(out.pop(), None);
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
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
            assert!(out.pop().is_none());
        }
        Ok(())
    }

    #[test]
    fn it_ends_with_both() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
            .tags(&[
                Tag::new(3, "burst", TagValue::Bool(true)),
                Tag::new(4, "test", TagValue::Bool(true)),
                Tag::new(7, "burst", TagValue::Bool(true)),
                Tag::new(7, "burst", TagValue::Bool(false)),
            ])
            .build()?;
        let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, 1);
        assert!(matches![src.work()?, BlockRet::EOF]);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (burst, tags) = out.pop().unwrap();
        assert_eq!(burst, &[4, 5, 6, 7, 8]);
        assert_eq!(tags, vec![Tag::new(1, "test", TagValue::Bool(true))]);
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn tags_in_tail() -> Result<()> {
        for (tags, want, want_extra_tags) in [
            // No tags in tail.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![],
            ),
            // Start in same as end.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                    Tag::new(4, "burst", TagValue::Bool(true)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![],
            ),
            // Start tag in tail.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                    Tag::new(5, "burst", TagValue::Bool(true)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![],
            ),
            // End tag in tail.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                    Tag::new(5, "burst", TagValue::Bool(false)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![],
            ),
            // Both tag in tail.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                    Tag::new(5, "burst", TagValue::Bool(false)),
                    Tag::new(5, "burst", TagValue::Bool(true)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![],
            ),
            // Unrelated tag in end tail.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                    Tag::new(4, "unrelated", TagValue::Bool(true)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![Tag::new(3, "unrelated", TagValue::Bool(true))],
            ),
            // Unrelated tag in tail.
            (
                vec![
                    Tag::new(1, "burst", TagValue::Bool(true)),
                    Tag::new(2, "test", TagValue::Bool(true)),
                    Tag::new(4, "burst", TagValue::Bool(false)),
                    Tag::new(5, "unrelated", TagValue::Bool(true)),
                ],
                vec![2, 3, 4, 5, 6, 7, 8],
                vec![Tag::new(4, "unrelated", TagValue::Bool(true))],
            ),
        ] {
            eprintln!("\n-=-=-=-=-=-=-=");
            eprintln!("Testing: {tags:?}");
            let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .tags(&tags)
                .build()?;
            let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, 4);
            assert!(matches![src.work()?, BlockRet::EOF]);
            assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
            let (burst, tags) = out.pop().unwrap();
            assert_eq!(burst, want);
            let mut want_tags = vec![Tag::new(1, "test", TagValue::Bool(true))];
            want_tags.extend(want_extra_tags);
            assert_eq!(tags, want_tags);
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
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (burst, tags) = out.pop().unwrap();
        assert_eq!(burst, &[4, 5, 6, 7]);
        assert_eq!(tags, vec![Tag::new(1, "test", TagValue::Bool(true))]);
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn just_end() -> Result<()> {
        let (mut src, src_out) = VectorSource::builder(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10])
            .tags(&[
                Tag::new(1, "test", TagValue::Bool(true)),
                Tag::new(2, "burst", TagValue::Bool(false)),
            ])
            .build()?;
        let (mut b, out) = StreamToPdu::new(src_out, "burst", 10, 0);
        assert!(matches![src.work()?, BlockRet::EOF]);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert!(out.pop().is_none());
        Ok(())
    }
}
