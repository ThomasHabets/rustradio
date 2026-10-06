//! Stream to PDU.
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagValue};
use crate::{Result, Sample};

#[derive(Default)]
enum State<T: Sample> {
    #[default]
    Unsync,
    Packet(Vec<T>, Vec<Tag>),
    Tail(Vec<T>, Vec<Tag>, usize),
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
/// Samples between bursts are discarded. Repeated start markers within a burst
/// do not restart it, and burst markers within the tail are ignored.
/// Other tags on included samples are forwarded with positions relative to the
/// PDU. Tags with the configured burst key are removed.
///
/// Bursts exceeding `max_size`, including their tail, are discarded. Incomplete
/// bursts or tails at end of input are not emitted.
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
    fn file_burst(&self, v: Vec<T>, tags: Vec<Tag>) {
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BurstTag {
    Start,
    End,
    Both,
}

// Find the next marker relevant to the current state. Bound the search by the
// span we can consume so tags beyond a size limit are not repeatedly scanned.
fn next_burst_tag(tags: &[Tag], key: &str, start: bool, limit: usize) -> Option<(usize, BurstTag)> {
    let (mut index, tag) = tags
        .iter()
        .enumerate()
        .take_while(|(_, tag)| tag.pos() < limit)
        .find(|(_, tag)| tag.key() == key && tag.val() == &TagValue::Bool(start))?;
    let pos = tag.pos();
    while index > 0 && tags[index - 1].pos() == pos {
        index -= 1;
    }
    let mut starts = false;
    let mut ends = false;
    for tag in tags[index..].iter().take_while(|tag| tag.pos() == pos) {
        if tag.key() == key {
            match tag.val() {
                TagValue::Bool(true) => starts = true,
                TagValue::Bool(false) => ends = true,
                _ => {}
            }
        }
    }
    let marker = match (starts, ends) {
        (true, true) => BurstTag::Both,
        (true, false) => BurstTag::Start,
        (false, true) => BurstTag::End,
        (false, false) => unreachable!("matched a Boolean burst tag"),
    };
    Some((pos, marker))
}

// Move only tags whose samples are included. The iterator also discards tags
// from skipped spans and control markers, without cloning their payloads.
fn append_span<T: Sample>(
    packet: &mut Vec<T>,
    packet_tags: &mut Vec<Tag>,
    input: &[T],
    tags: &mut std::vec::IntoIter<Tag>,
    key: &str,
    start: usize,
    end: usize,
) {
    let offset = packet.len();
    while tags.as_slice().first().is_some_and(|tag| tag.pos() < end) {
        let mut tag = tags.next().unwrap();
        if tag.pos() >= start && tag.key() != key {
            tag.set_pos(tag.pos() - start + offset);
            packet_tags.push(tag);
        }
    }
    packet.extend_from_slice(&input[start..end]);
}

impl<T: Sample> Block for StreamToPdu<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut output_space = self.dst.remaining();
        if output_space == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let (input, intags) = self.src.read_buf()?;
        let samples = input.slice();
        let mut tags = intags.into_iter();
        let mut pos = 0;
        while pos < samples.len() {
            // Move the packet state once per span rather than once per sample.
            match std::mem::take(&mut self.state) {
                State::Unsync => {
                    let Some((start, marker)) =
                        next_burst_tag(tags.as_slice(), &self.tag, true, samples.len())
                    else {
                        pos = samples.len();
                        break;
                    };
                    pos = start;
                    if marker == BurstTag::Both {
                        if self.tail == 0 {
                            self.file_burst(Vec::new(), Vec::new());
                            output_space -= 1;
                            pos += 1;
                        } else {
                            self.state = State::Tail(Vec::new(), Vec::new(), self.tail);
                        }
                    } else {
                        self.state = State::Packet(Vec::new(), Vec::new());
                    }
                }
                State::Packet(mut packet, mut packet_tags) => {
                    let remaining = self.max_size - packet.len();
                    let limit = pos + (samples.len() - pos).min(remaining.saturating_add(1));
                    let marker = next_burst_tag(tags.as_slice(), &self.tag, false, limit);
                    let end = marker.map_or(limit, |(end, _)| end);
                    if end == pos {
                        if self.tail == 0 {
                            self.file_burst(packet, packet_tags);
                            output_space -= 1;
                            pos += 1;
                        } else {
                            self.state = State::Tail(packet, packet_tags, self.tail);
                        }
                    } else if end - pos > remaining {
                        // Drop the oversized packet without copying the span.
                        // The sample that exceeds max_size is consumed too.
                        pos += remaining + 1;
                    } else {
                        append_span(
                            &mut packet,
                            &mut packet_tags,
                            samples,
                            &mut tags,
                            &self.tag,
                            pos,
                            end,
                        );
                        pos = end;
                        self.state = State::Packet(packet, packet_tags);
                    }
                }
                State::Tail(mut packet, mut packet_tags, remaining_tail) => {
                    let count = remaining_tail.min(samples.len() - pos);
                    let remaining = self.max_size - packet.len();
                    if count > remaining {
                        pos += remaining + 1;
                    } else {
                        append_span(
                            &mut packet,
                            &mut packet_tags,
                            samples,
                            &mut tags,
                            &self.tag,
                            pos,
                            pos + count,
                        );
                        pos += count;
                        if count == remaining_tail {
                            self.file_burst(packet, packet_tags);
                            output_space -= 1;
                        } else {
                            self.state = State::Tail(packet, packet_tags, remaining_tail - count);
                        }
                    }
                }
            }
            // Discard tags on excluded end samples and skipped/oversized spans.
            while tags.as_slice().first().is_some_and(|tag| tag.pos() < pos) {
                tags.next();
            }
            if output_space == 0 {
                input.consume(pos);
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
        }
        input.consume(pos);
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;
    use crate::blocks::VectorSource;

    fn feed(writer: &crate::stream::WriteStream<u8>, samples: &[u8], tags: &[Tag]) -> Result<()> {
        let mut window = writer.write_buf()?;
        window.fill_from_slice(samples);
        window.produce(samples.len(), tags);
        Ok(())
    }

    fn marker(pos: usize, value: bool) -> Tag {
        Tag::new(pos, "burst", TagValue::Bool(value))
    }

    fn metadata(pos: usize, value: u64) -> Tag {
        Tag::new(pos, "metadata", TagValue::U64(value))
    }

    #[test]
    fn ordered_spans_preserve_metadata_and_ignore_repeated_starts() -> Result<()> {
        for reversed in [false, true] {
            let (writer, reader) = crate::stream::new_stream();
            let (mut block, out) = StreamToPdu::new(reader, "burst", 30, 2);
            let mut tags = vec![
                metadata(0, 0),
                marker(3, true),
                metadata(3, 1),
                metadata(3, 2),
                metadata(4, 3),
                metadata(4, 4),
                Tag::new(5, "burst", TagValue::U64(1)),
                marker(6, true),
                marker(9, reversed),
                marker(9, !reversed),
                metadata(9, 5),
                marker(10, true),
                metadata(10, 6),
                metadata(11, 7),
                marker(12, true),
                marker(15, false),
                metadata(16, 8),
                metadata(17, 9),
            ];
            tags.sort_by_key(Tag::pos);
            feed(&writer, &(0..30).collect::<Vec<_>>(), &tags)?;
            block.work()?;
            assert_eq!(
                out.pop(),
                Some((
                    (3..11).collect(),
                    vec![
                        metadata(0, 1),
                        metadata(0, 2),
                        metadata(1, 3),
                        metadata(1, 4),
                        metadata(6, 5),
                        metadata(7, 6),
                    ]
                ))
            );
            assert_eq!(out.pop(), Some(((12..17).collect(), vec![metadata(4, 8)])));
            assert!(out.pop().is_none());
        }
        Ok(())
    }

    #[test]
    fn spans_and_tail_cross_input_windows() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream();
        let (mut block, out) = StreamToPdu::new(reader, "burst", 20, 4);
        feed(
            &writer,
            &[0, 1, 2, 3, 4, 5],
            &[marker(2, true), metadata(3, 1)],
        )?;
        block.work()?;
        assert!(out.pop().is_none());
        feed(
            &writer,
            &[6, 7, 8, 9],
            &[marker(2, false), metadata(2, 2), metadata(3, 3)],
        )?;
        block.work()?;
        assert!(out.pop().is_none());
        feed(
            &writer,
            &[10, 11, 12, 13],
            &[
                marker(0, true),
                metadata(0, 4),
                metadata(1, 5),
                metadata(2, 6),
            ],
        )?;
        block.work()?;
        assert_eq!(
            out.pop(),
            Some((
                (2..12).collect(),
                vec![
                    metadata(1, 1),
                    metadata(6, 2),
                    metadata(7, 3),
                    metadata(8, 4),
                    metadata(9, 5),
                ]
            ))
        );
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn simultaneous_markers_complete_at_eof_and_before_next_burst() -> Result<()> {
        for samples in [vec![42], vec![42, 43]] {
            for reversed in [false, true] {
                let (writer, reader) = crate::stream::new_stream();
                let (mut block, out) = StreamToPdu::new(reader, "burst", 1, 1);
                let tags: Vec<_> = (0..samples.len())
                    .flat_map(|pos| {
                        [
                            marker(pos, reversed),
                            marker(pos, !reversed),
                            metadata(pos, pos as u64),
                        ]
                    })
                    .collect();
                feed(&writer, &samples, &tags)?;
                drop(writer);
                block.work()?;
                for (pos, sample) in samples.iter().enumerate() {
                    assert_eq!(
                        out.pop(),
                        Some((vec![*sample], vec![metadata(0, pos as u64)]))
                    );
                }
                assert!(out.pop().is_none());
                assert!(matches!(block.state, State::Unsync));
            }
        }
        Ok(())
    }

    #[test]
    fn size_limit_resumes_after_the_offending_sample() -> Result<()> {
        let (writer, reader) = crate::stream::new_stream();
        let (mut block, out) = StreamToPdu::new(reader, "burst", 3, 0);
        feed(
            &writer,
            &(0..14).collect::<Vec<_>>(),
            &[
                marker(0, true),
                marker(3, true),
                marker(4, true),
                metadata(4, 1),
                marker(7, false),
                metadata(7, 2),
                marker(8, true),
                marker(10, false),
                metadata(10, 3),
            ],
        )?;
        block.work()?;
        assert_eq!(out.pop(), Some((vec![4, 5, 6], vec![metadata(0, 1)])));
        assert_eq!(out.pop(), Some((vec![8, 9], vec![])));
        assert!(out.pop().is_none());

        let (writer, reader) = crate::stream::new_stream();
        let (mut block, out) = StreamToPdu::new(reader, "burst", 0, 0);
        feed(
            &writer,
            &[0, 1, 2],
            &[
                marker(0, true),
                marker(1, true),
                marker(1, false),
                metadata(1, 1),
            ],
        )?;
        block.work()?;
        assert_eq!(out.pop(), Some((vec![], vec![])));
        assert!(out.pop().is_none());
        Ok(())
    }

    #[test]
    fn full_output_preserves_input_and_resumes_between_bursts() -> Result<()> {
        use crate::stream::StreamWait;
        let (writer, reader) = crate::stream::new_stream();
        let (mut block, out) = StreamToPdu::new(reader, "burst", 10, 1);
        feed(
            &writer,
            &(0..10).collect::<Vec<_>>(),
            &[
                marker(1, true),
                marker(3, false),
                metadata(3, 1),
                marker(4, true),
                metadata(4, 2),
                marker(6, false),
            ],
        )?;
        let input_id = block.src.id();
        let output_id = block.dst.id();
        let capacity = block.dst.remaining();
        for _ in 0..capacity {
            block.dst.push(vec![], vec![]);
        }
        assert!(matches!(block.work()?, BlockRet::WaitForStream(stream, 1)
            if stream.id() == output_id));
        let (window, _) = block.src.read_buf()?;
        assert_eq!(window.len(), 10);
        drop(window);
        assert_eq!(out.pop(), Some((vec![], vec![])));
        assert!(matches!(block.work()?, BlockRet::WaitForStream(stream, 1)
            if stream.id() == output_id));
        let (window, tags) = block.src.read_buf()?;
        assert_eq!(window.slice(), &[4, 5, 6, 7, 8, 9]);
        assert_eq!(tags[0], marker(0, true));
        drop(window);
        for _ in 1..capacity {
            assert_eq!(out.pop(), Some((vec![], vec![])));
        }
        assert_eq!(out.pop(), Some((vec![1, 2, 3], vec![metadata(2, 1)])));
        assert!(out.pop().is_none());
        assert!(matches!(block.work()?, BlockRet::WaitForStream(stream, 1)
            if stream.id() == input_id));
        assert_eq!(out.pop(), Some((vec![4, 5, 6], vec![metadata(0, 2)])));
        assert!(out.pop().is_none());
        Ok(())
    }

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
                assert_eq!(tags, want_tags);
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
