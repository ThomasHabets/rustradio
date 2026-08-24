//! Encode pulse-width-modulated OOK frames.
//!
//! [`PwmEncoder`] is the streaming inverse of [`crate::pwm_decoder::PwmDecoder`].
//! It consumes no-copy packets containing transmission-order bits (each byte
//! must be zero or one) and emits a normalized [`Float`] OOK envelope. A short
//! high pulse and a long high pulse encode the two bit values; the remainder
//! of each constant-width symbol is low.
//!
//! The encoder expands packets incrementally, so even very long pulse widths
//! do not require an equally large temporary allocation. Each packet becomes
//! one transmission containing a configurable number of identical frames,
//! followed by the configured reset gap. The repeat count can be changed at
//! runtime with [`PwmEncoderControl`]; changes take effect between packets.

use std::sync::mpsc;

use crate::block::{Block, BlockEOF, BlockRet};
use crate::pwm_decoder::PwmGapPulse;
use crate::stream::{NCReadStream, ReadStream, Tag, TagValue, WriteStream};
use crate::{Error, Float, Result};

/// Tag placed on the first sample of an encoded transmission.
///
/// Its value is the number of data bits in the frame.
pub const TAG_START: &str = "PwmEncoder::start";

/// Tag placed on the last sample of an encoded transmission.
///
/// Its value is the number of data bits in the frame.
pub const TAG_END: &str = "PwmEncoder::end";

/// Tag placed on the first sample of every repeated frame.
///
/// Its value is the zero-based repeat index.
pub const TAG_REPEAT: &str = "PwmEncoder::repeat";

/// Runtime control handle for [`PwmEncoder`].
///
/// Clones may be sent to threads that need to reconfigure an encoder while its
/// graph is running. Changes take effect when the encoder starts its next input
/// packet; a packet already being emitted keeps its original repeat count.
#[derive(Clone, Debug)]
pub struct PwmEncoderControl {
    tx: mpsc::Sender<PwmEncoderCommand>,
    repeated_frames_supported: bool,
}

#[derive(Debug)]
enum PwmEncoderCommand {
    Repeats(usize),
}

impl PwmEncoderControl {
    /// Set the number of identical frames emitted for each subsequent packet.
    ///
    /// # Errors
    ///
    /// Returns an error if `repeats` is zero, the encoder's frame and reset
    /// gaps cannot distinguish repeated frames, or the encoder has been
    /// dropped.
    pub fn set_repeats(&self, repeats: usize) -> Result<()> {
        validate_repeat_count(repeats, self.repeated_frames_supported)?;
        self.tx
            .send(PwmEncoderCommand::Repeats(repeats))
            .map_err(|error| Error::wrap(error, "PwmEncoder control channel"))
    }
}

/// Validate a repeat count against the configured frame/reset gap relationship.
fn validate_repeat_count(repeats: usize, repeated_frames_supported: bool) -> Result<()> {
    if repeats == 0 {
        return Err(Error::msg(
            "PwmEncoder repeat count must be greater than zero",
        ));
    }
    if repeats > 1 && !repeated_frames_supported {
        return Err(Error::msg(
            "PwmEncoder reset gap must be greater than the frame gap when emitting repeated frames",
        ));
    }
    Ok(())
}

/// Validated settings retained by [`PwmEncoder`].
#[derive(Clone, Debug)]
struct PwmEncoderSettings {
    short_width: usize,
    long_width: usize,
    frame_gap: usize,
    reset_gap: usize,
    repeats: usize,
    max_frame_bits: usize,
    short_is_one: bool,
    gap_pulse: PwmGapPulse,
    low_level: Float,
    high_level: Float,
}

impl PwmEncoderSettings {
    /// Create settings with the builder's documented defaults.
    fn new(short_width: usize, long_width: usize, frame_gap: usize, reset_gap: usize) -> Self {
        Self {
            short_width,
            long_width,
            frame_gap,
            reset_gap,
            repeats: 1,
            max_frame_bits: 4_096,
            short_is_one: true,
            gap_pulse: PwmGapPulse::default(),
            low_level: 0.0,
            high_level: 1.0,
        }
    }

    /// Reject settings that cannot produce an unambiguous decoder input.
    fn validate(&self) -> Result<()> {
        if self.short_width == 0 {
            return Err(Error::msg(
                "PwmEncoder short pulse width must be greater than zero",
            ));
        }
        if self.long_width <= self.short_width {
            return Err(Error::msg(
                "PwmEncoder long pulse width must be greater than the short pulse width",
            ));
        }
        if self.frame_gap <= self.long_width {
            return Err(Error::msg(
                "PwmEncoder frame gap must be greater than the long pulse width",
            ));
        }
        if self.reset_gap < self.frame_gap {
            return Err(Error::msg(
                "PwmEncoder reset gap must be at least as long as the frame gap",
            ));
        }
        validate_repeat_count(self.repeats, self.reset_gap > self.frame_gap)?;
        if self.max_frame_bits == 0 {
            return Err(Error::msg(
                "PwmEncoder maximum frame length must be greater than zero",
            ));
        }
        if !(0.0..=1.0).contains(&self.low_level)
            || !(0.0..=1.0).contains(&self.high_level)
            || self.low_level >= self.high_level
        {
            return Err(Error::msg(
                "PwmEncoder levels must be finite, between zero and one, and low must be less than high",
            ));
        }
        Ok(())
    }

    /// Return the high-pulse width for one binary value.
    fn high_width(&self, bit: u8) -> usize {
        if (bit == 1) == self.short_is_one {
            self.short_width
        } else {
            self.long_width
        }
    }

    /// Return the low portion of the constant-width symbol for one bit.
    fn low_width(&self, bit: u8) -> usize {
        if self.high_width(bit) == self.short_width {
            self.long_width
        } else {
            self.short_width
        }
    }

    /// Return the number of output samples in each repeated frame.
    fn frame_samples(&self, bits: &[u8]) -> Result<usize> {
        let symbol_width = self
            .short_width
            .checked_add(self.long_width)
            .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))?;
        let symbols = bits
            .len()
            .checked_mul(symbol_width)
            .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))?;
        let gap = match self.gap_pulse {
            PwmGapPulse::Data => self.frame_gap - self.low_width(bits[bits.len() - 1]),
            PwmGapPulse::Delimiter => self
                .short_width
                .checked_add(self.frame_gap)
                .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))?,
        };
        symbols
            .checked_add(gap)
            .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))
    }

    /// Return the total samples emitted for one input packet.
    fn transmission_samples(&self, bits: &[u8]) -> Result<(usize, usize)> {
        let frame_samples = self.frame_samples(bits)?;
        let frames = frame_samples
            .checked_mul(self.repeats)
            .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))?;
        let reset_tail = self.reset_gap - self.frame_gap;
        let total = frames
            .checked_add(reset_tail)
            .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))?;
        Ok((frame_samples, total))
    }
}

/// Builder for [`PwmEncoder`].
#[derive(Clone, Debug)]
pub struct PwmEncoderBuilder {
    settings: PwmEncoderSettings,
}

impl PwmEncoderBuilder {
    /// Initialize a builder from the required protocol timings.
    fn new(short_width: usize, long_width: usize, frame_gap: usize, reset_gap: usize) -> Self {
        Self {
            settings: PwmEncoderSettings::new(short_width, long_width, frame_gap, reset_gap),
        }
    }

    /// Set the number of identical frames emitted for every input packet.
    #[must_use]
    pub fn repeats(mut self, repeats: usize) -> Self {
        self.settings.repeats = repeats;
        self
    }

    /// Set the maximum accepted input packet length in bits.
    #[must_use]
    pub fn max_frame_bits(mut self, bits: usize) -> Self {
        self.settings.max_frame_bits = bits;
        self
    }

    /// Choose whether a nominal short pulse represents one or zero.
    #[must_use]
    pub fn short_is_one(mut self, short_is_one: bool) -> Self {
        self.settings.short_is_one = short_is_one;
        self
    }

    /// Choose whether the pulse immediately before a frame gap is data.
    ///
    /// In [`PwmGapPulse::Delimiter`] mode the encoder inserts a short high
    /// pulse after the data bits. A correspondingly configured decoder drops
    /// that pulse rather than treating it as another bit.
    #[must_use]
    pub fn gap_pulse(mut self, gap_pulse: PwmGapPulse) -> Self {
        self.settings.gap_pulse = gap_pulse;
        self
    }

    /// Set the low and high output levels.
    ///
    /// The defaults are zero and one. Both values must be finite and between
    /// zero and one, and `low` must be less than `high`.
    #[must_use]
    pub fn levels(mut self, low: Float, high: Float) -> Self {
        self.settings.low_level = low;
        self.settings.high_level = high;
        self
    }

    /// Validate the settings and build an encoder connected to `src`.
    ///
    /// # Errors
    ///
    /// Returns an error if the pulse widths, gaps, repeat count, frame limit,
    /// or output levels cannot form a valid encoder configuration.
    pub fn build(self, src: NCReadStream<Vec<u8>>) -> Result<(PwmEncoder, ReadStream<Float>)> {
        self.settings.validate()?;
        Ok(PwmEncoder::from_settings(src, self.settings))
    }
}

/// Current run within one expanded input packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    BitHigh,
    BitLow,
    DelimiterHigh,
    FrameGap,
    ResetGap,
    Done,
}

/// Incremental state for one packet being expanded into samples.
#[derive(Debug)]
struct ActiveTransmission {
    bits: Vec<u8>,
    tags: Vec<Tag>,
    next_tag: usize,
    repeat: usize,
    bit: usize,
    phase: Phase,
    remaining: usize,
    offset: usize,
}

impl ActiveTransmission {
    /// Validate one packet and initialize its first high pulse and tags.
    fn new(bits: Vec<u8>, input_tags: Vec<Tag>, settings: &PwmEncoderSettings) -> Result<Self> {
        if bits.is_empty() {
            return Err(Error::msg("PwmEncoder input frame must not be empty"));
        }
        if bits.len() > settings.max_frame_bits {
            return Err(Error::msg(format!(
                "PwmEncoder input frame has {} bits, exceeding the configured maximum of {}",
                bits.len(),
                settings.max_frame_bits,
            )));
        }
        if bits.iter().any(|&bit| bit > 1) {
            return Err(Error::msg(
                "PwmEncoder input frames may contain only zero and one",
            ));
        }

        let symbol_width = settings
            .short_width
            .checked_add(settings.long_width)
            .ok_or_else(|| Error::msg("PwmEncoder sample count overflow"))?;
        let (frame_samples, total_samples) = settings.transmission_samples(&bits)?;
        let bit_count = u64::try_from(bits.len())?;
        let mut tags = Vec::with_capacity(input_tags.len() + settings.repeats + 2);
        tags.push(Tag::new(0, TAG_START, TagValue::U64(bit_count)));
        for tag in input_tags {
            if tag.pos() < bits.len() {
                let pos = tag
                    .pos()
                    .checked_mul(symbol_width)
                    .ok_or_else(|| Error::msg("PwmEncoder tag position overflow"))?;
                tags.push(Tag::new(pos, tag.key(), tag.val().clone()));
            }
        }
        for repeat in 0..settings.repeats {
            let pos = repeat
                .checked_mul(frame_samples)
                .ok_or_else(|| Error::msg("PwmEncoder tag position overflow"))?;
            tags.push(Tag::new(
                pos,
                TAG_REPEAT,
                TagValue::U64(u64::try_from(repeat)?),
            ));
        }
        tags.push(Tag::new(
            total_samples - 1,
            TAG_END,
            TagValue::U64(bit_count),
        ));
        tags.sort_by_key(Tag::pos);

        Ok(Self {
            remaining: settings.high_width(bits[0]),
            bits,
            tags,
            next_tag: 0,
            repeat: 0,
            bit: 0,
            phase: Phase::BitHigh,
            offset: 0,
        })
    }

    /// Return whether the packet has been fully expanded.
    fn done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Return the output value for the current run.
    fn level(&self, settings: &PwmEncoderSettings) -> Float {
        match self.phase {
            Phase::BitHigh | Phase::DelimiterHigh => settings.high_level,
            Phase::BitLow | Phase::FrameGap | Phase::ResetGap => settings.low_level,
            Phase::Done => settings.low_level,
        }
    }

    /// Move to the next non-empty output run.
    fn advance(&mut self, settings: &PwmEncoderSettings) {
        match self.phase {
            Phase::BitHigh => {
                self.phase = Phase::BitLow;
                self.remaining = settings.low_width(self.bits[self.bit]);
            }
            Phase::BitLow if self.bit + 1 < self.bits.len() => {
                self.bit += 1;
                self.phase = Phase::BitHigh;
                self.remaining = settings.high_width(self.bits[self.bit]);
            }
            Phase::BitLow => match settings.gap_pulse {
                PwmGapPulse::Data => {
                    self.phase = Phase::FrameGap;
                    self.remaining = settings.frame_gap - settings.low_width(self.bits[self.bit]);
                }
                PwmGapPulse::Delimiter => {
                    self.phase = Phase::DelimiterHigh;
                    self.remaining = settings.short_width;
                }
            },
            Phase::DelimiterHigh => {
                self.phase = Phase::FrameGap;
                self.remaining = settings.frame_gap;
            }
            Phase::FrameGap if self.repeat + 1 < settings.repeats => {
                self.repeat += 1;
                self.bit = 0;
                self.phase = Phase::BitHigh;
                self.remaining = settings.high_width(self.bits[0]);
            }
            Phase::FrameGap => {
                let reset_tail = settings.reset_gap - settings.frame_gap;
                if reset_tail == 0 {
                    self.phase = Phase::Done;
                    self.remaining = 0;
                } else {
                    self.phase = Phase::ResetGap;
                    self.remaining = reset_tail;
                }
            }
            Phase::ResetGap => {
                self.phase = Phase::Done;
                self.remaining = 0;
            }
            Phase::Done => {}
        }
    }

    /// Fill as much of `output` as possible and translate covered tags.
    fn write(
        &mut self,
        output: &mut [Float],
        output_tags: &mut Vec<Tag>,
        settings: &PwmEncoderSettings,
    ) -> usize {
        let start = self.offset;
        let mut written = 0;
        while written < output.len() && !self.done() {
            let n = self.remaining.min(output.len() - written);
            output[written..written + n].fill(self.level(settings));
            written += n;
            self.remaining -= n;
            self.offset += n;
            if self.remaining == 0 {
                self.advance(settings);
            }
        }

        while self.next_tag < self.tags.len() && self.tags[self.next_tag].pos() < self.offset {
            let tag = &self.tags[self.next_tag];
            if tag.pos() >= start {
                output_tags.push(Tag::new(tag.pos() - start, tag.key(), tag.val().clone()));
            }
            self.next_tag += 1;
        }
        written
    }
}

/// Expand packets of bits into a pulse-width-modulated OOK envelope.
///
/// Input packets contain bytes equal to zero or one in transmission order.
/// Output samples use the configured low and high levels, with a variable-rate
/// relationship determined by the bits and pulse settings. Each packet is
/// repeated as one transmission and terminated by `reset_gap` consecutive low
/// samples.
///
/// Packet tags whose positions name valid input bits are translated to the
/// start of those bits in the first repeat. Tags beyond the packet are dropped.
/// [`TAG_START`], [`TAG_REPEAT`], and [`TAG_END`] describe the expanded output.
///
/// # Example
///
/// ```
/// use rustradio::blocks::{PwmEncoder, PwmGapPulse};
/// use rustradio::stream::new_nocopy_stream;
///
/// let (_packets, input) = new_nocopy_stream::<Vec<u8>>();
/// let (encoder, _samples) = PwmEncoder::builder(25, 80, 110, 914)
///     .repeats(3)
///     .gap_pulse(PwmGapPulse::Delimiter)
///     .build(input)?;
/// encoder.control().set_repeats(5)?;
/// # Ok::<(), rustradio::Error>(())
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct PwmEncoder {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
    settings: PwmEncoderSettings,
    active: Option<ActiveTransmission>,
    command_rx: mpsc::Receiver<PwmEncoderCommand>,
    control_tx: PwmEncoderControl,
}

impl PwmEncoder {
    /// Create a PWM encoder builder.
    ///
    /// Required protocol parameters are expressed in output samples:
    ///
    /// - `short_width`: nominal duration of a short high pulse.
    /// - `long_width`: nominal duration of a long high pulse.
    /// - `frame_gap`: consecutive low samples that end one frame. It must be
    ///   greater than `long_width`, so an in-symbol low cannot end the frame.
    /// - `reset_gap`: consecutive low samples that end a transmission. It must
    ///   be at least `frame_gap`.
    ///
    /// By default one frame is emitted, at most 4,096 bits are accepted, short
    /// pulses represent one, the last data pulse precedes the frame gap, and
    /// the output levels are zero and one.
    #[must_use]
    pub fn builder(
        short_width: usize,
        long_width: usize,
        frame_gap: usize,
        reset_gap: usize,
    ) -> PwmEncoderBuilder {
        PwmEncoderBuilder::new(short_width, long_width, frame_gap, reset_gap)
    }

    /// Create an encoder from validated settings.
    fn from_settings(
        src: NCReadStream<Vec<u8>>,
        settings: PwmEncoderSettings,
    ) -> (Self, ReadStream<Float>) {
        let (dst, dst_read) = crate::stream::new_stream();
        let (command_tx, command_rx) = mpsc::channel();
        let control_tx = PwmEncoderControl {
            tx: command_tx,
            repeated_frames_supported: settings.reset_gap > settings.frame_gap,
        };
        (
            Self {
                src,
                dst,
                settings,
                active: None,
                command_rx,
                control_tx,
            },
            dst_read,
        )
    }

    /// Return a control handle for changing settings while the graph is running.
    #[must_use]
    pub fn control(&self) -> PwmEncoderControl {
        self.control_tx.clone()
    }

    /// Apply all queued changes before starting a new packet.
    fn apply_pending_commands(&mut self) {
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                PwmEncoderCommand::Repeats(repeats) => self.settings.repeats = repeats,
            }
        }
    }
}

impl BlockEOF for PwmEncoder {
    /// Report EOF only after an active packet has fully drained.
    fn eof(&mut self) -> bool {
        self.active.is_none() && self.src.eof()
    }
}

impl Block for PwmEncoder {
    /// Expand one packet at a time while honoring output backpressure.
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut output = self.dst.write_buf()?;
        if output.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }

        if self.active.is_none() {
            self.apply_pending_commands();
            let Some((bits, tags)) = self.src.pop() else {
                return if self.src.eof() {
                    Ok(BlockRet::EOF)
                } else {
                    Ok(BlockRet::WaitForStream(&self.src, 1))
                };
            };
            self.active = Some(ActiveTransmission::new(bits, tags, &self.settings)?);
        }

        let mut tags = Vec::new();
        let n = self.active.as_mut().expect("active was initialized").write(
            output.slice(),
            &mut tags,
            &self.settings,
        );
        debug_assert_ne!(n, 0);
        let done = self.active.as_ref().is_some_and(ActiveTransmission::done);
        output.produce(n, &tags);
        if done {
            self.active = None;
            if self.src.eof() {
                Ok(BlockRet::EOF)
            } else {
                Ok(BlockRet::Again)
            }
        } else {
            Ok(BlockRet::WaitForStream(&self.dst, 1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pwm_decoder::{PwmDecoder, PwmFrame};
    use crate::stream::{DEFAULT_STREAM_SIZE, TagValue, new_nocopy_stream};

    const SHORT: usize = 2;
    const LONG: usize = 5;
    const FRAME_GAP: usize = 9;
    const RESET_GAP: usize = 20;

    /// Construct the compact encoder builder used by the synthetic tests.
    fn builder() -> PwmEncoderBuilder {
        PwmEncoder::builder(SHORT, LONG, FRAME_GAP, RESET_GAP).max_frame_bits(64)
    }

    /// Run one packet through an encoder and collect all samples and tags.
    fn encode(
        bits: Vec<u8>,
        input_tags: &[Tag],
        builder: PwmEncoderBuilder,
    ) -> Result<(Vec<Float>, Vec<Tag>)> {
        let (input, input_read) = new_nocopy_stream();
        input.push(bits, input_tags);
        drop(input);
        let (mut encoder, output) = builder.build(input_read)?;
        drain_encoder(&mut encoder, &output)
    }

    /// Drain an encoder whose input has been closed, collecting samples and tags.
    fn drain_encoder(
        encoder: &mut PwmEncoder,
        output: &ReadStream<Float>,
    ) -> Result<(Vec<Float>, Vec<Tag>)> {
        let mut samples = Vec::new();
        let mut tags = Vec::new();
        loop {
            let ret = encoder.work()?;
            let (buffer, buffer_tags) = output.read_buf()?;
            let offset = samples.len();
            samples.extend_from_slice(buffer.slice());
            tags.extend(
                buffer_tags
                    .into_iter()
                    .map(|tag| Tag::new(offset + tag.pos(), tag.key(), tag.val().clone())),
            );
            let n = buffer.len();
            buffer.consume(n);
            if matches!(ret, BlockRet::EOF) {
                break;
            }
        }
        Ok((samples, tags))
    }

    /// Consume currently buffered output and summarize its repeat tags.
    fn consume_output(
        output: &ReadStream<Float>,
        samples: &mut usize,
        repeats: &mut Vec<u64>,
    ) -> Result<()> {
        let (buffer, tags) = output.read_buf()?;
        *samples += buffer.len();
        repeats.extend(tags.into_iter().filter_map(|tag| {
            if tag.key() == TAG_REPEAT
                && let TagValue::U64(repeat) = tag.val()
            {
                return Some(*repeat);
            }
            None
        }));
        let n = buffer.len();
        buffer.consume(n);
        Ok(())
    }

    /// Decode generated samples with the public inverse block.
    fn decode(samples: &[Float], bits: usize, repeats: usize) -> Result<Vec<PwmFrame>> {
        let input = crate::stream::ReadStream::from_slice(samples);
        let (mut decoder, output) = PwmDecoder::builder(0.5, SHORT, LONG, FRAME_GAP, RESET_GAP)
            .frame_bits(Some(bits))
            .min_repeats(repeats)
            .gap_pulse(PwmGapPulse::Delimiter)
            .build(input)?;
        loop {
            if matches!(decoder.work()?, BlockRet::EOF) {
                break;
            }
        }
        let mut frames = Vec::new();
        while let Some((frame, _)) = output.pop() {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Verify the exact run layout for data-terminated frames.
    #[test]
    fn data_gap_waveform() -> Result<()> {
        let (samples, _) = encode(vec![1, 0], &[], builder().levels(0.25, 0.75))?;
        let mut want = Vec::new();
        want.extend(std::iter::repeat_n(0.75, SHORT));
        want.extend(std::iter::repeat_n(0.25, LONG));
        want.extend(std::iter::repeat_n(0.75, LONG));
        want.extend(std::iter::repeat_n(0.25, FRAME_GAP));
        want.extend(std::iter::repeat_n(0.25, RESET_GAP - FRAME_GAP));
        assert_eq!(samples, want);
        Ok(())
    }

    /// Verify delimiter frames round-trip through the matching decoder.
    #[test]
    fn delimiter_frames_round_trip_with_repeats() -> Result<()> {
        let bits = (0..25)
            .map(|index| u8::from(index % 3 == 0))
            .collect::<Vec<_>>();
        let (samples, _) = encode(
            bits.clone(),
            &[],
            builder()
                .max_frame_bits(25)
                .repeats(3)
                .gap_pulse(PwmGapPulse::Delimiter),
        )?;
        let frames = decode(&samples, bits.len(), 3)?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bits(), bits);
        assert_eq!(frames[0].repeats(), 3);
        Ok(())
    }

    /// Verify runtime changes are applied before the next packet starts.
    #[test]
    fn runtime_repeat_change_applies_to_next_packet() -> Result<()> {
        let bits = vec![1, 0, 1, 1];
        let (input, input_read) = new_nocopy_stream();
        input.push(bits.clone(), &[]);
        drop(input);
        let (mut encoder, output) = builder()
            .gap_pulse(PwmGapPulse::Delimiter)
            .build(input_read)?;
        let control = encoder.control();
        control.set_repeats(2)?;
        control.set_repeats(3)?;

        let (samples, tags) = drain_encoder(&mut encoder, &output)?;
        let repeats = tags
            .iter()
            .filter(|tag| tag.key() == TAG_REPEAT)
            .map(Tag::val)
            .collect::<Vec<_>>();
        assert_eq!(
            repeats,
            vec![&TagValue::U64(0), &TagValue::U64(1), &TagValue::U64(2)]
        );
        let frames = decode(&samples, bits.len(), 3)?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bits(), bits);
        assert_eq!(frames[0].repeats(), 3);
        Ok(())
    }

    /// Verify an active packet retains its original repeat count.
    #[test]
    fn runtime_repeat_change_waits_for_active_packet() -> Result<()> {
        let long = DEFAULT_STREAM_SIZE + 5;
        let frame_gap = long + 1;
        let reset_gap = frame_gap + 3;
        let mut first_settings = PwmEncoderSettings::new(1, long, frame_gap, reset_gap);
        let first_samples = first_settings.transmission_samples(&[0])?.1;
        first_settings.repeats = 2;
        let second_samples = first_settings.transmission_samples(&[0])?.1;

        let (input, input_read) = new_nocopy_stream();
        input.push(vec![0], &[]);
        input.push(vec![0], &[]);
        drop(input);
        let (mut encoder, output) =
            PwmEncoder::builder(1, long, frame_gap, reset_gap).build(input_read)?;
        let control = encoder.control();

        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        let mut samples = 0;
        let mut repeat_tags = Vec::new();
        consume_output(&output, &mut samples, &mut repeat_tags)?;
        assert_eq!(repeat_tags, vec![0]);

        control.set_repeats(2)?;
        loop {
            let ret = encoder.work()?;
            consume_output(&output, &mut samples, &mut repeat_tags)?;
            if matches!(ret, BlockRet::EOF) {
                break;
            }
        }
        assert_eq!(samples, first_samples + second_samples);
        assert_eq!(repeat_tags, vec![0, 0, 1]);
        Ok(())
    }

    /// Verify the control handle rejects repeat counts invalid for this encoder.
    #[test]
    fn runtime_repeat_change_validates_count_and_gaps() -> Result<()> {
        let (_input, input_read) = new_nocopy_stream();
        let (encoder, _) = builder().build(input_read)?;
        let control = encoder.control();
        assert!(control.set_repeats(0).is_err());
        control.set_repeats(2)?;

        let (_input, input_read) = new_nocopy_stream();
        let (encoder, _) = PwmEncoder::builder(2, 5, 9, 9).build(input_read)?;
        let control = encoder.control();
        control.set_repeats(1)?;
        assert!(control.set_repeats(2).is_err());
        Ok(())
    }

    /// Verify bit polarity can be inverted on both sides of a round trip.
    #[test]
    fn inverted_pulse_polarity() -> Result<()> {
        let bits = vec![1, 0, 1, 1];
        let (samples, _) = encode(bits.clone(), &[], builder().short_is_one(false))?;
        let input = crate::stream::ReadStream::from_slice(&samples);
        let (mut decoder, output) = PwmDecoder::builder(0.5, SHORT, LONG, FRAME_GAP, RESET_GAP)
            .frame_bits(Some(bits.len()))
            .short_is_one(false)
            .build(input)?;
        assert!(matches!(decoder.work()?, BlockRet::EOF));
        assert_eq!(output.pop().expect("frame").0.bits(), bits);
        Ok(())
    }

    /// Verify packet tags are translated and generated boundary tags exist.
    #[test]
    fn tags_follow_first_repeat_bits() -> Result<()> {
        let input_tag = Tag::new(1, "input", TagValue::Bool(true));
        let ignored_tag = Tag::new(4, "ignored", TagValue::Bool(true));
        let (samples, tags) = encode(
            vec![1, 0, 1, 0],
            &[input_tag, ignored_tag],
            builder().repeats(2),
        )?;
        assert!(tags.contains(&Tag::new(0, TAG_START, TagValue::U64(4))));
        assert!(tags.contains(&Tag::new(0, TAG_REPEAT, TagValue::U64(0))));
        assert!(tags.contains(&Tag::new(SHORT + LONG, "input", TagValue::Bool(true),)));
        assert!(!tags.iter().any(|tag| tag.key() == "ignored"));
        assert!(tags.contains(&Tag::new(samples.len() - 1, TAG_END, TagValue::U64(4),)));
        Ok(())
    }

    /// Verify an active long run survives output backpressure and drains.
    #[test]
    fn output_backpressure_preserves_long_runs() -> Result<()> {
        let long = DEFAULT_STREAM_SIZE + 5;
        let frame_gap = long + 1;
        let reset_gap = frame_gap + 3;
        let (input, input_read) = new_nocopy_stream();
        input.push(vec![0], &[]);
        drop(input);
        let (mut encoder, output) =
            PwmEncoder::builder(1, long, frame_gap, reset_gap).build(input_read)?;

        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(!BlockEOF::eof(&mut encoder));
        let mut samples = Vec::new();
        loop {
            let (buffer, _) = output.read_buf()?;
            samples.extend_from_slice(buffer.slice());
            let n = buffer.len();
            buffer.consume(n);
            if matches!(encoder.work()?, BlockRet::EOF) {
                let (buffer, _) = output.read_buf()?;
                samples.extend_from_slice(buffer.slice());
                let n = buffer.len();
                buffer.consume(n);
                break;
            }
        }
        assert!(BlockEOF::eof(&mut encoder));
        assert_eq!(samples.len(), long + reset_gap);
        assert!(samples[..long].iter().all(|&sample| sample == 1.0));
        assert!(samples[long..].iter().all(|&sample| sample == 0.0));
        Ok(())
    }

    /// Verify empty input waits while open and reaches EOF once closed.
    #[test]
    fn empty_input_waits_then_reaches_eof() -> Result<()> {
        let (input, input_read) = new_nocopy_stream();
        let (mut encoder, _) = builder().build(input_read)?;
        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        drop(input);
        assert!(matches!(encoder.work()?, BlockRet::EOF));
        assert!(BlockEOF::eof(&mut encoder));
        Ok(())
    }

    /// Verify invalid settings and packets fail explicitly.
    #[test]
    fn validates_settings_and_packets() -> Result<()> {
        let input = || new_nocopy_stream::<Vec<u8>>().1;
        assert!(PwmEncoder::builder(0, 5, 9, 20).build(input()).is_err());
        assert!(PwmEncoder::builder(5, 5, 9, 20).build(input()).is_err());
        assert!(PwmEncoder::builder(2, 5, 5, 20).build(input()).is_err());
        assert!(PwmEncoder::builder(2, 5, 9, 8).build(input()).is_err());
        assert!(builder().repeats(0).build(input()).is_err());
        assert!(
            PwmEncoder::builder(2, 5, 9, 9)
                .repeats(2)
                .build(input())
                .is_err()
        );
        assert!(builder().max_frame_bits(0).build(input()).is_err());
        assert!(builder().levels(Float::NAN, 1.0).build(input()).is_err());
        assert!(builder().levels(-0.1, 1.0).build(input()).is_err());
        assert!(builder().levels(0.0, 1.1).build(input()).is_err());
        assert!(builder().levels(1.0, 1.0).build(input()).is_err());

        for bits in [vec![], vec![0, 2, 1], vec![0; 65]] {
            let (sender, receiver) = new_nocopy_stream();
            sender.push(bits, &[]);
            let (mut encoder, _) = builder().build(receiver)?;
            assert!(encoder.work().is_err());
        }
        Ok(())
    }
}
