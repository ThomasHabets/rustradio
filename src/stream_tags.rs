//! Tag accounting shared by native and WASM stream buffers.
use std::collections::VecDeque;

use crate::stream::Tag;

/// Sample offset from the beginning of a stream, independent of buffer wrapping.
///
/// Kept separate from `TagPos`, which is relative to the current window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct AbsoluteStreamPos(u64);

impl AbsoluteStreamPos {
    fn advance(self, samples: usize) -> Self {
        Self(
            self.0
                .checked_add(u64::try_from(samples).expect("sample count exceeds u64"))
                .expect("absolute stream position overflow"),
        )
    }

    fn relative_to(self, start: Self) -> usize {
        usize::try_from(self.0.checked_sub(start.0).expect("tag precedes reader"))
            .expect("relative tag position exceeds usize")
    }
}

/// Tags stay in stream order even when the underlying sample buffer wraps.
/// Writes append tags and consumption removes them from the front.
#[derive(Debug, Default)]
pub(crate) struct StreamTags {
    read_pos: AbsoluteStreamPos,
    write_pos: AbsoluteStreamPos,
    tags: VecDeque<(AbsoluteStreamPos, Tag)>,
}

impl StreamTags {
    pub(crate) fn produce(&mut self, samples: usize, tags: &[Tag]) {
        let end = self.write_pos.advance(samples);
        // An out-of-range tag could overlap a later batch and break FIFO order.
        assert!(
            tags.iter().all(|tag| tag.pos() < samples),
            "tag outside produced samples"
        );
        let previous_len = self.tags.len();
        self.tags.extend(
            tags.iter()
                .map(|tag| (self.write_pos.advance(tag.pos()), tag.clone())),
        );
        // The API accepts unsorted input. Sort only the new batch when needed,
        // preserving the producer's order for tags at the same sample.
        if !tags.windows(2).all(|pair| pair[0].pos() <= pair[1].pos()) {
            self.tags.make_contiguous()[previous_len..].sort_by_key(|(pos, _)| *pos);
        }
        self.write_pos = end;
    }

    pub(crate) fn consume(&mut self, samples: usize) {
        let end = self.read_pos.advance(samples);
        assert!(end <= self.write_pos, "consumed beyond produced samples");
        while self.tags.front().is_some_and(|(pos, _)| *pos < end) {
            self.tags.pop_front();
        }
        self.read_pos = end;
    }

    pub(crate) fn read(&self) -> Vec<Tag> {
        self.tags
            .iter()
            .map(|(pos, tag)| {
                let mut tag = tag.clone();
                tag.set_pos(pos.relative_to(self.read_pos));
                tag
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::TagValue;

    #[test]
    fn tags_beyond_32_bit_positions_remain_relative_to_the_reader() {
        let mut tags = StreamTags::default();
        let advance = u32::MAX as usize;
        tags.produce(advance, &[]);
        tags.consume(advance);
        tags.produce(8, &[Tag::new(5, "marker", TagValue::Bool(true))]);
        tags.consume(3);
        assert_eq!(tags.read(), [Tag::new(2, "marker", TagValue::Bool(true))]);
    }

    #[test]
    #[should_panic(expected = "absolute stream position overflow")]
    fn absolute_position_cannot_silently_wrap() {
        AbsoluteStreamPos(u64::MAX).advance(1);
    }

    #[test]
    #[should_panic(expected = "tag outside produced samples")]
    fn tag_cannot_overlap_the_next_batch() {
        StreamTags::default().produce(1, &[Tag::new(1, "marker", TagValue::Bool(true))]);
    }
}
