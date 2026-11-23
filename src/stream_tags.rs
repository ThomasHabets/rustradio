//! Tag accounting shared by native and WASM stream buffers.
use std::borrow::Cow;
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

/// Unread tags owned by one read window. Positions remain absolute until exposed.
///
/// This is where the tags live while the [`BufferReader`](crate::sys::BufferReader) exists. The tags are
/// either:
/// * handed back to the stream, if the [`BufferReader`](crate::sys::BufferReader) is dropped without
///   being consumed,
/// * returned in the [`BufferReader::consume()`](crate::sys::BufferReader::consume) call,
/// * dropped by [`BufferReader::consume_discard_tags()`](crate::sys::BufferReader::consume_discard_tags),
/// * a mix of handed back and returned/dropped, if the `n` passed to the
///   consumption function is less than or equal to a tag's relative position.
///
/// It may seem strange to take them out of the stream and either put back in or
/// later returned by `consume()`, but it's to prevent string copies of tag
/// names and string tag values, without locking the underlying stream's
/// `VecDeque` of tags.
///
/// Maybe a future solution is to have a smarter container in the underlying
/// stream than a `VecDeque`, that allows the reading of tags without lock,
/// allowing writers to add tags concurrently and having the reader only lock
/// the container to drain consumed tags.
#[derive(Debug, Default)]
pub(crate) struct ReadWindowTags {
    start: AbsoluteStreamPos,
    entries: VecDeque<(AbsoluteStreamPos, Tag)>,
}

impl ReadWindowTags {
    // The caller owns the detached queue, so rebasing needs no stream lock.
    pub(crate) fn rebased(mut self) -> Self {
        for (pos, tag) in &mut self.entries {
            tag.set_pos(pos.relative_to(self.start));
        }
        self
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &Tag> + Clone {
        self.entries.iter().map(|(_, tag)| tag)
    }

    pub(crate) fn drain_prefix(&mut self, samples: usize) -> impl Iterator<Item = Tag> + '_ {
        let start = self.start;
        let end = start.advance(samples);
        let count = if self.entries.back().is_none_or(|(pos, _)| *pos < end) {
            self.entries.len()
        } else {
            self.entries
                .iter()
                .take_while(|(pos, _)| *pos < end)
                .count()
        };
        self.start = end;
        self.entries.drain(..count).map(|(_, tag)| tag)
    }
}

/// Tags stay in stream order even when the underlying sample buffer wraps.
/// Writes append tags and consumption removes them from the front.
#[derive(Debug, Default)]
pub(crate) struct StreamTags {
    // Returned by the last reader. Writers append only to the pending queue.
    unread: ReadWindowTags,
    write_pos: AbsoluteStreamPos,
    pending: VecDeque<(AbsoluteStreamPos, Tag)>,
}

impl StreamTags {
    pub(crate) fn position(&self) -> u64 {
        self.write_pos.0
    }

    pub(crate) fn produce(&mut self, samples: usize, tags: Cow<'_, [Tag]>) {
        let end = self.write_pos.advance(samples);
        // An out-of-range tag could overlap a later batch and break FIFO order.
        assert!(
            tags.iter().all(|tag| tag.pos() < samples),
            "tag outside produced samples"
        );
        let sorted = tags.windows(2).all(|pair| pair[0].pos() <= pair[1].pos());
        let previous_len = self.pending.len();
        match tags {
            Cow::Borrowed(tags) => self.append(tags.iter().cloned()),
            Cow::Owned(tags) => self.append(tags),
        }
        // The API accepts unsorted input. Sort only the new batch when needed,
        // preserving the producer's order for tags at the same sample.
        //
        // TODO: wait, no. We should not accept unsorted input.
        if !sorted {
            self.pending.make_contiguous()[previous_len..].sort_by_key(|(pos, _)| *pos);
        }
        self.write_pos = end;
    }

    fn append(&mut self, tags: impl IntoIterator<Item = Tag>) {
        self.pending.extend(
            tags.into_iter()
                .map(|tag| (self.write_pos.advance(tag.pos()), tag)),
        );
    }

    pub(crate) fn checkout(&mut self) -> ReadWindowTags {
        // Keep the unconsumed suffix in place. Only append newly written tags.
        if self.unread.entries.is_empty() {
            std::mem::swap(&mut self.unread.entries, &mut self.pending);
        } else {
            self.unread.entries.append(&mut self.pending);
        }
        std::mem::take(&mut self.unread)
    }

    pub(crate) fn restore(&mut self, tags: ReadWindowTags) {
        debug_assert!(self.unread.entries.is_empty());
        self.unread = tags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::TagValue;

    #[test]
    fn owned_tags_move_string_allocations_and_preserve_stable_order() {
        let mut tags = StreamTags::default();
        tags.produce(10, Cow::Borrowed(&[]));
        let mut window = tags.checkout();
        drop(window.drain_prefix(10));
        tags.restore(window);
        let owned = Tag::new(3, "owned key", TagValue::String("owned value".into()));
        let key_ptr = owned.key().as_ptr();
        let TagValue::String(value) = owned.val() else {
            unreachable!();
        };
        let value_ptr = value.as_ptr();
        tags.produce(
            5,
            Cow::Owned(vec![
                owned,
                Tag::new(1, "first", TagValue::Bool(true)),
                Tag::new(1, "second", TagValue::Bool(false)),
            ]),
        );
        let stored = &tags.pending[2];
        assert_eq!(stored.0, AbsoluteStreamPos(13));
        assert_eq!(stored.1.key().as_ptr(), key_ptr);
        let TagValue::String(value) = stored.1.val() else {
            unreachable!();
        };
        assert_eq!(value.as_ptr(), value_ptr);
        let mut window = tags.checkout();
        drop(window.drain_prefix(1));
        tags.restore(window);
        assert_eq!(
            tags.checkout()
                .rebased()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [
                Tag::new(0, "first", TagValue::Bool(true)),
                Tag::new(0, "second", TagValue::Bool(false)),
                Tag::new(2, "owned key", TagValue::String("owned value".into())),
            ]
        );
    }

    #[test]
    #[should_panic(expected = "tag outside produced samples")]
    fn owned_tag_cannot_overlap_the_next_batch() {
        StreamTags::default().produce(
            1,
            Cow::Owned(vec![Tag::new(1, "marker", TagValue::Bool(true))]),
        );
    }

    #[test]
    fn tags_beyond_32_bit_positions_remain_relative_to_the_reader() {
        let mut tags = StreamTags::default();
        let advance = u32::MAX as usize;
        tags.produce(advance, Cow::Borrowed(&[]));
        let mut window = tags.checkout();
        drop(window.drain_prefix(advance));
        tags.restore(window);
        tags.produce(
            8,
            Cow::Borrowed(&[Tag::new(5, "marker", TagValue::Bool(true))]),
        );
        let mut window = tags.checkout();
        drop(window.drain_prefix(3));
        tags.restore(window);
        assert_eq!(
            tags.checkout()
                .rebased()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [Tag::new(2, "marker", TagValue::Bool(true))]
        );
    }

    #[test]
    #[should_panic(expected = "absolute stream position overflow")]
    fn absolute_position_cannot_silently_wrap() {
        AbsoluteStreamPos(u64::MAX).advance(1);
    }

    #[test]
    #[should_panic(expected = "tag outside produced samples")]
    fn tag_cannot_overlap_the_next_batch() {
        StreamTags::default().produce(
            1,
            Cow::Borrowed(&[Tag::new(1, "marker", TagValue::Bool(true))]),
        );
    }
}
