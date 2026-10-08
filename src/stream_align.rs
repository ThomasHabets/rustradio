//! Align two sample streams using tagged absolute positions.
use crate::block::{Block, BlockEOF, BlockRet};
use crate::iq_stream::{ABSOLUTE_SAMPLE_INDEX, GAP_SAMPLES, SAMPLE_INDEX};
use crate::stream::{ReadStream, StreamWait, Tag, TagValue, WriteStream, new_stream};
use crate::{Error, Result, Sample};

/// Configure the position tag independently for each input.
pub struct StreamAlignBuilder<T: Sample, U: Sample> {
    left: ReadStream<T>,
    right: ReadStream<U>,
    left_key: String,
    right_key: String,
}
impl<T: Sample, U: Sample> StreamAlignBuilder<T, U> {
    /// Set position keys. Pass the same key twice to configure both inputs alike.
    #[must_use]
    pub fn tag_keys(mut self, left: impl Into<String>, right: impl Into<String>) -> Self {
        self.left_key = left.into();
        self.right_key = right.into();
        self
    }
    /// Build the block and its two outputs. Keys must be nonempty and cannot
    /// equal the reserved IqStream gap or session-position keys.
    // Keep the constructor tuple consistent with other blocks with two outputs.
    #[allow(clippy::type_complexity)]
    pub fn build(self) -> Result<(StreamAlign<T, U>, ReadStream<T>, ReadStream<U>)> {
        for key in [&self.left_key, &self.right_key] {
            if key.is_empty() || matches!(key.as_str(), GAP_SAMPLES | SAMPLE_INDEX) {
                return Err(Error::msg("invalid StreamAlign position key"));
            }
        }
        Ok(StreamAlign::with_keys(
            self.left,
            self.right,
            self.left_key,
            self.right_key,
        ))
    }
}

/// Produce paired samples at the same absolute position, dropping unmatched data.
///
/// Inputs may have different sample types, but must share a sample rate and
/// position reference. This block does not resample or correct clock drift.
/// Each input must carry a `U64` position tag on its first sample. The default
/// key is [`ABSOLUTE_SAMPLE_INDEX`]; use the builder to choose other keys.
///
/// Position tags and [`GAP_SAMPLES`] are honored throughout the stream. A forward
/// jump drops samples from the other input until positions match. Backward jumps,
/// conflicting position tags, malformed markers, and counter overflow are errors.
/// Empty inputs end the block without output; either exhausted input ends both
/// outputs after all available matched pairs. Unmatched tails are discarded.
///
/// Ordinary tags are retained separately on their output, in their original
/// order, except tags on discarded samples. Position and gap markers are replaced
/// with consistent output markers: both outputs carry their configured position
/// key at startup and discontinuities, and equal gap counts after output begins.
/// [`SAMPLE_INDEX`] on a gap is relative to the first output absolute position,
/// including gaps. Initial alignment drops do not produce an output gap.
///
/// ```
/// use rustradio::blocks::StreamAlign;
/// use rustradio::stream::new_stream;
/// let (_, left) = new_stream::<f32>();
/// let (_, right) = new_stream::<rustradio::Complex>();
/// let (align, left_out, right_out) = StreamAlign::new(left, right);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct StreamAlign<T: Sample, U: Sample> {
    #[rustradio(in)]
    left: ReadStream<T>,
    #[rustradio(in)]
    right: ReadStream<U>,
    #[rustradio(out)]
    left_out: WriteStream<T>,
    #[rustradio(out)]
    right_out: WriteStream<U>,
    left_position: Position,
    right_position: Position,
    // Absolute index of the first emitted pair; output session indices use it
    // as their origin even after subsequent gaps.
    origin: Option<u64>,
    // Exclusive absolute end of the last emitted batch, used to measure the
    // combined loss caused by input gaps and alignment drops.
    output_end: Option<u64>,
    done: bool,
}

// Markers at the current buffer head must be applied once, even when repeated
// work calls cannot consume it because the other input or an output is blocked.
struct Position {
    key: String,
    // Absolute index of the next input sample, unknown until its first anchor.
    next: Option<u64>,
    head_processed: bool,
    anchor_at_head: bool,
}
impl Position {
    /// Start without a known position; the first sample must establish it.
    fn new(key: String) -> Self {
        Self {
            key,
            next: None,
            head_processed: false,
            anchor_at_head: false,
        }
    }
    /// Apply markers on the next input sample once. Tags are ordered, so only
    /// the position-zero prefix matters. A colocated anchor includes the gap;
    /// without an anchor, advance the known position by the reported loss.
    fn at_head(&mut self, tags: &[Tag]) -> Result<()> {
        if self.head_processed {
            return Ok(());
        }
        let mut anchor = None;
        let mut gap = None;
        for tag in tags.iter().take_while(|tag| tag.pos() == 0) {
            if tag.key() != self.key && tag.key() != GAP_SAMPLES {
                continue;
            }
            let TagValue::U64(value) = tag.val() else {
                return Err(Error::msg("StreamAlign position and gap tags must be U64"));
            };
            if tag.key() == self.key {
                if anchor.is_some_and(|previous| previous != *value) {
                    return Err(Error::msg("conflicting StreamAlign position tags"));
                }
                anchor = Some(*value);
            } else if *value == 0 || gap.replace(*value).is_some() {
                return Err(Error::msg("invalid or duplicate StreamAlign gap tag"));
            }
        }
        let expected = self
            .next
            .map(|next| checked_add(next, gap.unwrap_or(0)))
            .transpose()?;
        if let Some(anchor) = anchor {
            if expected.is_some_and(|next| anchor < next) {
                return Err(Error::msg("StreamAlign position moved backwards"));
            }
            // An absolute anchor already includes any loss reported at this
            // sample, including a gap before the first sample of a connection.
            self.next = Some(anchor);
        } else {
            self.next = Some(expected.ok_or_else(|| {
                Error::msg("StreamAlign requires a position tag on the first sample")
            })?);
        }
        self.anchor_at_head = anchor.is_some();
        self.head_processed = true;
        Ok(())
    }
    /// Limit a batch to samples before the next position or gap marker. Copying
    /// and dropping both stop here so the next marker can update the cursor.
    fn span(&self, tags: &[Tag], available: usize) -> usize {
        tags.iter()
            .find(|tag| tag.pos() > 0 && (tag.key() == self.key || tag.key() == GAP_SAMPLES))
            .map_or(available, |tag| tag.pos().min(available))
    }
    /// Commit a checked cursor advance after samples have been copied or
    /// dropped. The new head's markers must be processed on the next iteration.
    fn consumed(&mut self, next: u64) {
        self.next = Some(next);
        self.head_processed = false;
        self.anchor_at_head = false;
    }
    /// Replace input timeline markers with the shared output timeline while
    /// retaining ordinary tags covered by this batch. `anchor` is absolute;
    /// `gap` contains (missing samples, position relative to the output origin).
    /// Generated markers precede ordinary tags at the same sample position.
    fn output_tags(
        &self,
        tags: Vec<Tag>,
        count: usize,
        anchor: Option<u64>,
        gap: Option<(u64, u64)>,
    ) -> Vec<Tag> {
        let mut output = Vec::new();
        if let Some(index) = anchor {
            output.push(Tag::new(0, &self.key, TagValue::U64(index)));
        }
        if let Some((missing, index)) = gap {
            output.push(Tag::new(0, GAP_SAMPLES, TagValue::U64(missing)));
            output.push(Tag::new(0, SAMPLE_INDEX, TagValue::U64(index)));
        }
        output.extend(
            tags.into_iter()
                .take_while(|tag| tag.pos() < count)
                .filter(|tag| {
                    tag.key() != self.key && !matches!(tag.key(), GAP_SAMPLES | SAMPLE_INDEX)
                }),
        );
        output
    }
}
/// Advance without allowing a wrapped cursor to look like a backward jump.
fn checked_add(position: u64, count: u64) -> Result<u64> {
    position
        .checked_add(count)
        .ok_or_else(|| Error::msg("StreamAlign position overflow"))
}
impl<T: Sample, U: Sample> StreamAlign<T, U> {
    /// Create an aligner using the standard absolute-position tag on both inputs.
    #[must_use]
    pub fn new(left: ReadStream<T>, right: ReadStream<U>) -> (Self, ReadStream<T>, ReadStream<U>) {
        Self::with_keys(
            left,
            right,
            ABSOLUTE_SAMPLE_INDEX.into(),
            ABSOLUTE_SAMPLE_INDEX.into(),
        )
    }
    /// Configure position keys before creating the block and outputs.
    #[must_use]
    pub fn builder(left: ReadStream<T>, right: ReadStream<U>) -> StreamAlignBuilder<T, U> {
        StreamAlignBuilder {
            left,
            right,
            left_key: ABSOLUTE_SAMPLE_INDEX.into(),
            right_key: ABSOLUTE_SAMPLE_INDEX.into(),
        }
    }
    /// Allocate the two independent outputs and initialize their shared timeline.
    /// Public construction paths supply either default or validated custom keys.
    fn with_keys(
        left: ReadStream<T>,
        right: ReadStream<U>,
        left_key: String,
        right_key: String,
    ) -> (Self, ReadStream<T>, ReadStream<U>) {
        let (left_out, left_read) = new_stream();
        let (right_out, right_read) = new_stream();
        (
            Self {
                left,
                right,
                left_out,
                right_out,
                left_position: Position::new(left_key),
                right_position: Position::new(right_key),
                origin: None,
                output_end: None,
                done: false,
            },
            left_read,
            right_read,
        )
    }
}
impl<T: Sample, U: Sample> BlockEOF for StreamAlign<T, U> {
    /// Let work drain matched input before declaring completion. Input closure
    /// alone does not mean the buffered samples have already been emitted.
    fn eof(&mut self) -> bool {
        self.done
    }
}
impl<T: Sample, U: Sample> Block for StreamAlign<T, U> {
    /// Process whole spans until input or output readiness blocks progress.
    /// Dropping advances the lower cursor; copying advances both equally and
    /// consumes input only after both outputs can accept the matched batch.
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            // Check EOF before checking out read buffers, which hold additional
            // references and would obscure whether the producer has closed.
            if self.done
                || self.left.eof()
                || self.right.eof()
                || self.left_out.closed()
                || self.right_out.closed()
            {
                self.done = true;
                return Ok(BlockRet::EOF);
            }

            let (left, left_tags) = self.left.read_buf()?;
            if left.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.left, 1));
            }
            let (right, right_tags) = self.right.read_buf()?;
            if right.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.right, 1));
            }

            self.left_position.at_head(&left_tags)?;
            self.right_position.at_head(&right_tags)?;
            let lp = self.left_position.next.expect("validated position");
            let rp = self.right_position.next.expect("validated position");
            let left_span = self.left_position.span(&left_tags, left.len());
            let right_span = self.right_position.span(&right_tags, right.len());
            // Discard lower-index samples until the cursors meet, but stop at
            // intervening markers which may change the next alignment target.
            if lp < rp {
                let count = (rp - lp).min(left_span as u64) as usize;
                let next = checked_add(lp, count as u64)?;
                left.consume(count);
                self.left_position.consumed(next);
                continue;
            }
            if rp < lp {
                let count = (lp - rp).min(right_span as u64) as usize;
                let next = checked_add(rp, count as u64)?;
                right.consume(count);
                self.right_position.consumed(next);
                continue;
            }
            // Both cursors now identify the same sample. Capacity on either
            // output limits the pair, regardless of the input sample types.
            let mut left_out = self.left_out.write_buf()?;
            if left_out.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.left_out, 1));
            }
            let mut right_out = self.right_out.write_buf()?;
            if right_out.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.right_out, 1));
            }
            let count = left_span
                .min(right_span)
                .min(left_out.len())
                .min(right_out.len());
            let end = checked_add(lp, count as u64)?;
            let origin = self.origin.unwrap_or(lp);
            // Measure loss since the last emitted pair, rather than forwarding
            // one input's gap count: realignment may discard additional samples.
            // Startup has no previous batch, so initial skips are not a gap.
            let gap = self
                .output_end
                .filter(|previous| *previous < lp)
                .map(|previous| (lp - previous, lp - origin));
            let anchor = if self.output_end != Some(lp)
                || self.left_position.anchor_at_head
                || self.right_position.anchor_at_head
            {
                Some(lp)
            } else {
                None
            };
            let left_tags = self
                .left_position
                .output_tags(left_tags, count, anchor, gap);
            let right_tags = self
                .right_position
                .output_tags(right_tags, count, anchor, gap);
            left_out.fill_from_slice(&left.slice()[..count]);
            right_out.fill_from_slice(&right.slice()[..count]);
            left_out.produce(count, &left_tags);
            right_out.produce(count, &right_tags);
            left.consume(count);
            right.consume(count);
            self.left_position.consumed(end);
            self.right_position.consumed(end);
            self.origin = Some(origin);
            self.output_end = Some(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Make a position tag whose buffer offset and absolute index may differ.
    fn anchor(pos: usize, index: u64) -> Tag {
        Tag::new(pos, ABSOLUTE_SAMPLE_INDEX, TagValue::U64(index))
    }
    /// Mark missing timeline samples immediately before the tagged sample.
    fn gap(pos: usize, missing: u64) -> Tag {
        Tag::new(pos, GAP_SAMPLES, TagValue::U64(missing))
    }
    /// Populate a finite input and close its producer so tests can exercise EOF.
    fn input<T: Sample>(samples: &[T], tags: &[Tag]) -> Result<ReadStream<T>> {
        let (write, read) = new_stream();
        let mut buffer = write.write_buf()?;
        buffer.fill_from_slice(samples);
        buffer.produce(samples.len(), tags);
        drop(write);
        Ok(read)
    }
    /// Inspect queued output without consuming it, allowing separate tag checks.
    fn samples<T: Sample>(stream: &ReadStream<T>) -> Result<Vec<T>> {
        Ok(stream.read_buf()?.0.slice().to_vec())
    }
    /// Inspect tags relative to the same unconsumed output window as samples().
    fn tags<T: Sample>(stream: &ReadStream<T>) -> Result<Vec<Tag>> {
        Ok(stream.read_buf()?.1)
    }

    #[test]
    fn starts_and_different_types() -> Result<()> {
        for (left_start, right_start, want_left, want_right) in [
            (100, 100, vec![0u32, 1, 2, 3], vec![0.0f32, 1.0, 2.0, 3.0]),
            (100, 102, vec![2, 3], vec![0.0, 1.0]),
            (102, 100, vec![0, 1], vec![2.0, 3.0]),
        ] {
            let left = input(&[0u32, 1, 2, 3], &[anchor(0, left_start)])?;
            let right = input(&[0.0f32, 1.0, 2.0, 3.0], &[anchor(0, right_start)])?;
            let (mut block, out_left, out_right) = StreamAlign::new(left, right);
            assert!(matches!(block.work()?, BlockRet::EOF));
            assert_eq!(samples(&out_left)?, want_left);
            assert_eq!(samples(&out_right)?, want_right);
            let expected = vec![anchor(0, left_start.max(right_start))];
            assert_eq!(tags(&out_left)?, expected);
            assert_eq!(tags(&out_right)?, expected);
        }
        Ok(())
    }

    #[test]
    fn custom_keys_and_tag_translation() -> Result<()> {
        let left = input(
            &[10u32, 11, 12, 13],
            &[
                Tag::new(0, "left-position", TagValue::U64(10)),
                Tag::new(0, "discard", TagValue::Bool(true)),
                Tag::new(2, "a", TagValue::Bool(true)),
                Tag::new(2, "a", TagValue::Bool(false)),
                Tag::new(3, "b", TagValue::U64(3)),
            ],
        )?;
        let right = input(
            &[12f32, 13.0],
            &[
                Tag::new(0, "right-position", TagValue::U64(12)),
                Tag::new(1, "c", TagValue::U64(1)),
            ],
        )?;
        let (mut block, l, r) = StreamAlign::builder(left, right)
            .tag_keys("left-position", "right-position")
            .build()?;
        block.work()?;
        assert_eq!(
            tags(&l)?,
            vec![
                Tag::new(0, "left-position", TagValue::U64(12)),
                Tag::new(0, "a", TagValue::Bool(true)),
                Tag::new(0, "a", TagValue::Bool(false)),
                Tag::new(1, "b", TagValue::U64(3)),
            ]
        );
        assert_eq!(
            tags(&r)?,
            vec![
                Tag::new(0, "right-position", TagValue::U64(12)),
                Tag::new(1, "c", TagValue::U64(1))
            ]
        );
        Ok(())
    }

    #[test]
    fn gaps_on_either_input_and_colocated_anchors() -> Result<()> {
        for swap in [false, true] {
            for with_anchor in [false, true] {
                let mut markers = vec![anchor(0, 100), gap(2, 3)];
                if with_anchor {
                    markers.push(anchor(2, 105));
                }
                let sparse = input(&[0u32, 1, 5, 6, 7], &markers)?;
                let dense = input(&[0u32, 1, 2, 3, 4, 5, 6, 7, 8], &[anchor(0, 100)])?;
                let (left, right) = if swap {
                    (dense, sparse)
                } else {
                    (sparse, dense)
                };
                let (mut block, l, r) = StreamAlign::new(left, right);
                assert!(matches!(block.work()?, BlockRet::EOF));
                assert_eq!(samples(&l)?, [0, 1, 5, 6, 7]);
                assert_eq!(samples(&r)?, [0, 1, 5, 6, 7]);
                let expected = vec![
                    anchor(0, 100),
                    anchor(2, 105),
                    gap(2, 3),
                    Tag::new(2, SAMPLE_INDEX, TagValue::U64(5)),
                ];
                assert_eq!(tags(&l)?, expected);
                assert_eq!(tags(&r)?, expected);
            }
        }
        Ok(())
    }

    #[test]
    fn gaps_on_both_inputs_merge_into_shared_timeline() -> Result<()> {
        let left = input(&[0u32, 1, 5, 6, 7], &[anchor(0, 100), gap(2, 3)])?;
        let right = input(&[0u32, 3, 4, 5, 6, 7], &[anchor(0, 100), gap(1, 2)])?;
        let (mut block, l, r) = StreamAlign::new(left, right);
        block.work()?;
        assert_eq!(samples(&l)?, [0, 5, 6, 7]);
        assert_eq!(samples(&r)?, [0, 5, 6, 7]);
        assert_eq!(
            tags(&l)?,
            vec![
                anchor(0, 100),
                anchor(1, 105),
                gap(1, 4),
                Tag::new(1, SAMPLE_INDEX, TagValue::U64(5))
            ]
        );
        assert_eq!(tags(&l)?, tags(&r)?);
        Ok(())
    }

    #[test]
    fn later_position_jump_and_large_u64_positions() -> Result<()> {
        let start = u64::MAX - 20;
        let left = input(&[0u32, 5, 6], &[anchor(0, start), anchor(1, start + 5)])?;
        let right = input(&[0u32, 1, 2, 3, 4, 5, 6], &[anchor(0, start)])?;
        let (mut block, l, r) = StreamAlign::new(left, right);
        block.work()?;
        assert_eq!(samples(&l)?, [0, 5, 6]);
        assert_eq!(samples(&l)?, samples(&r)?);
        assert_eq!(
            tags(&l)?,
            vec![
                anchor(0, start),
                anchor(1, start + 5),
                gap(1, 4),
                Tag::new(1, SAMPLE_INDEX, TagValue::U64(5))
            ]
        );
        Ok(())
    }

    #[test]
    fn malformed_markers_and_overflow() -> Result<()> {
        for bad in [
            vec![],
            vec![anchor(1, 0)],
            vec![Tag::new(0, ABSOLUTE_SAMPLE_INDEX, TagValue::I64(0))],
            vec![anchor(0, 0), anchor(0, 1)],
            vec![anchor(0, 5), anchor(1, 4)],
            vec![anchor(0, 0), Tag::new(1, GAP_SAMPLES, TagValue::Bool(true))],
            vec![anchor(0, 0), gap(1, 0)],
            vec![anchor(0, 0), gap(1, 1), gap(1, 1)],
            vec![anchor(0, u64::MAX)],
            vec![anchor(0, u64::MAX - 1), gap(1, 2)],
        ] {
            let right_start = bad
                .iter()
                .find_map(|tag| match (tag.pos(), tag.val()) {
                    (0, TagValue::U64(value)) if tag.key() == ABSOLUTE_SAMPLE_INDEX => Some(*value),
                    _ => None,
                })
                .unwrap_or(0);
            let (mut block, l, r) = StreamAlign::new(
                input(&[1u32, 2], &bad)?,
                input(&[1u32, 2], &[anchor(0, right_start)])?,
            );
            assert!(block.work().is_err(), "markers: {bad:?}");
            drop((l, r));
        }
        Ok(())
    }

    #[test]
    fn invalid_configuration() {
        for key in ["", GAP_SAMPLES, SAMPLE_INDEX] {
            let (_, left) = new_stream::<u32>();
            let (_, right) = new_stream::<f32>();
            assert!(
                StreamAlign::builder(left, right)
                    .tag_keys(key, ABSOLUTE_SAMPLE_INDEX)
                    .build()
                    .is_err()
            );
        }
    }

    #[test]
    fn empty_and_no_overlap_end_without_output() -> Result<()> {
        for (left_values, left_start) in [(vec![], 0), (vec![0u32, 1], 0)] {
            let markers = if left_values.is_empty() {
                vec![]
            } else {
                vec![anchor(0, left_start)]
            };
            let (mut block, l, r) = StreamAlign::new(
                input(&left_values, &markers)?,
                input(&[10u32, 11], &[anchor(0, 10)])?,
            );
            assert!(matches!(block.work()?, BlockRet::EOF));
            assert!(samples(&l)?.is_empty());
            assert!(samples(&r)?.is_empty());
        }
        Ok(())
    }

    #[test]
    fn repeated_wait_does_not_apply_a_gap_twice() -> Result<()> {
        let (right_write, right) = new_stream::<u32>();
        let left = input(&[5u32, 6], &[anchor(0, 100), gap(0, 5)])?;
        let (mut block, l, r) = StreamAlign::new(left, right);
        for _ in 0..3 {
            assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
        }
        let mut buffer = right_write.write_buf()?;
        buffer.fill_from_slice(&[5u32, 6]);
        buffer.produce(2, &[anchor(0, 100)]);
        drop(right_write);
        block.work()?;
        assert_eq!(samples(&l)?, [5, 6]);
        assert_eq!(samples(&r)?, [5, 6]);
        assert_eq!(tags(&l)?, vec![anchor(0, 100)]);
        Ok(())
    }

    #[test]
    fn backpressure_preserves_paired_input_and_gap_state() -> Result<()> {
        for block_left in [false, true] {
            let left = input(&[0u32, 4, 5], &[anchor(0, 0), gap(1, 3)])?;
            let right = input(&[0u32, 1, 2, 3, 4, 5], &[anchor(0, 0)])?;
            let (mut block, l, r) = StreamAlign::new(left, right);
            let writer = if block_left {
                &block.left_out
            } else {
                &block.right_out
            };
            let mut buffer = writer.write_buf()?;
            let capacity = buffer.len();
            buffer.slice().fill(999);
            buffer.produce(capacity, &[]);
            for _ in 0..3 {
                assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
            }
            assert_eq!(block.left_position.next, Some(0));
            assert_eq!(block.right_position.next, Some(0));
            let blocked = if block_left { &l } else { &r };
            blocked.read_buf()?.0.consume(capacity);
            assert!(matches!(block.work()?, BlockRet::EOF));
            assert_eq!(samples(&l)?, [0, 4, 5]);
            assert_eq!(samples(&r)?, [0, 4, 5]);
            assert_eq!(
                tags(&l)?,
                vec![
                    anchor(0, 0),
                    anchor(1, 4),
                    gap(1, 3),
                    Tag::new(1, SAMPLE_INDEX, TagValue::U64(4))
                ]
            );
            assert_eq!(tags(&l)?, tags(&r)?);
        }
        Ok(())
    }
    #[test]
    fn incremental_inputs_and_gap_while_output_is_full() -> Result<()> {
        let (left_write, left) = new_stream::<u32>();
        let (right_write, right) = new_stream::<u32>();
        let (mut block, l, r) = StreamAlign::new(left, right);
        let mut initial = left_write.write_buf()?;
        initial.fill_from_slice(&[0]);
        initial.produce(1, &[anchor(0, 0)]);
        assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
        let mut initial = right_write.write_buf()?;
        initial.fill_from_slice(&[0]);
        initial.produce(1, &[anchor(0, 0)]);
        let mut filler = block.left_out.write_buf()?;
        let capacity = filler.len();
        filler.slice()[..capacity - 1].fill(999);
        filler.produce(capacity - 1, &[]);
        block.work()?; // One pair fills the left output.
        let mut more = left_write.write_buf()?;
        more.fill_from_slice(&[4, 5]);
        more.produce(2, &[gap(0, 3), anchor(0, 4)]);
        let mut more = right_write.write_buf()?;
        more.fill_from_slice(&[1, 2, 3, 4, 5]);
        more.produce(5, &[]);
        drop((left_write, right_write));
        for _ in 0..3 {
            assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
            assert_eq!(block.left_position.next, Some(4));
            assert_eq!(block.right_position.next, Some(4));
        }
        let (buffer, _) = l.read_buf()?;
        assert_eq!(buffer.slice()[capacity - 1], 0);
        buffer.consume(capacity);
        assert!(matches!(block.work()?, BlockRet::EOF));
        assert_eq!(samples(&l)?, [4, 5]);
        assert_eq!(samples(&r)?, [0, 4, 5]);
        assert_eq!(
            tags(&l)?,
            vec![
                anchor(0, 4),
                gap(0, 3),
                Tag::new(0, SAMPLE_INDEX, TagValue::U64(4))
            ]
        );
        assert_eq!(
            tags(&r)?,
            vec![
                anchor(0, 0),
                anchor(1, 4),
                gap(1, 3),
                Tag::new(1, SAMPLE_INDEX, TagValue::U64(4))
            ]
        );
        Ok(())
    }

    #[test]
    fn closed_output_stops_both_streams() -> Result<()> {
        let (mut block, l, r) = StreamAlign::new(
            input(&[1u32, 2], &[anchor(0, 0)])?,
            input(&[1u32, 2], &[anchor(0, 0)])?,
        );
        drop(l);
        assert!(matches!(block.work()?, BlockRet::EOF));
        assert!(samples(&r)?.is_empty());
        assert!(block.eof());
        Ok(())
    }

    #[test]
    fn alignment_across_successive_input_buffers() -> Result<()> {
        let (write, left) = new_stream::<u32>();
        let right = input(&[5u32, 6, 7], &[anchor(0, 5)])?;
        let (mut block, l, r) = StreamAlign::new(left, right);
        for chunk in 0..4 {
            let mut buffer = write.write_buf()?;
            buffer.fill_from_slice(&[chunk * 2, chunk * 2 + 1]);
            buffer.produce(
                2,
                &if chunk == 0 {
                    vec![anchor(0, 0)]
                } else {
                    vec![]
                },
            );
            block.work()?;
        }
        drop(write);
        assert!(matches!(block.work()?, BlockRet::EOF));
        assert_eq!(samples(&l)?, [5, 6, 7]);
        assert_eq!(samples(&r)?, [5, 6, 7]);
        assert_eq!(tags(&l)?, vec![anchor(0, 5)]);
        Ok(())
    }
}
