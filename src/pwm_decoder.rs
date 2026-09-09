//! Decode pulse-width-modulated OOK frames.
//!
//! [`PwmDecoder`] thresholds a [`Float`] power stream, measures
//! high-pulse widths, and emits frames separated by a configurable low-gap
//! threshold.
//! Timings are expressed in input samples, so callers can derive them from the
//! sample rate and protocol timings without coupling the block to either.
//!
//! A fixed-size protocol can set [`PwmDecoderBuilder::frame_bits`] to
//! `Some(bits)`. With `None`, any non-empty frame terminated by the configured
//! frame gap is accepted. Source EOF flushes already completed frames, but is
//! not itself considered a frame boundary: a partial final pulse train is
//! discarded.
//!
//! Each decoded frame also reports the mean carrier-on and carrier-off power
//! of its accepted repeats. [`PwmFrame::snr_db`] returns their ratio in
//! decibels: `10 * log10(carrier_on_power / carrier_off_power)`. This is an OOK
//! level ratio: carrier-on power includes noise and is not noise-subtracted.

use std::collections::VecDeque;

use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagValue};
use crate::{Error, Float, Result};

/// Output tag containing [`PwmFrame::snr_db`] as a [`TagValue::Float`].
///
/// The tag is placed at position zero of each no-copy frame output and omitted
/// when the carrier-on or carrier-off mean is zero or non-finite.
pub const TAG_SNR_DB: &str = "PwmDecoder::snr_db";

/// How to handle the high pulse immediately before a frame gap.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PwmGapPulse {
    /// Decode the pulse as the final bit in the frame.
    #[default]
    Data,

    /// Discard the pulse because it is a frame delimiter, not data.
    Delimiter,
}

/// Validated settings retained by [`PwmDecoder`].
#[derive(Clone, Debug)]
struct PwmDecoderSettings {
    /// Power threshold for changing from the low state to the high state.
    high_threshold: Float,

    /// Power threshold for changing from the high state to the low state.
    low_threshold: Float,

    /// Nominal width of a short high pulse, measured in input samples.
    short_width: usize,

    /// Nominal width of a long high pulse, measured in input samples.
    long_width: usize,

    /// Maximum distance from either nominal pulse width, in input samples.
    pulse_tolerance: usize,

    /// Low-gap threshold after which a frame is terminated.
    frame_gap: usize,

    /// Low-gap threshold after which a transmission is terminated.
    reset_gap: usize,

    /// Required bits per frame, or `None` to accept gap-delimited lengths.
    frame_bits: Option<usize>,

    /// Maximum bits retained for one frame before it is rejected.
    max_frame_bits: usize,

    /// Maximum distinct frame patterns tracked within one transmission.
    max_distinct_frames: usize,

    /// Identical copies required before a frame is emitted.
    min_repeats: usize,

    /// Whether a short pulse decodes as one rather than zero.
    short_is_one: bool,

    /// How to interpret the high pulse immediately before a frame gap.
    gap_pulse: PwmGapPulse,
}

impl PwmDecoderSettings {
    /// Create settings with the builder's documented defaults.
    fn new(
        high_threshold: Float,
        short_width: usize,
        long_width: usize,
        frame_gap: usize,
        reset_gap: usize,
    ) -> Self {
        Self {
            high_threshold,
            low_threshold: high_threshold / 2.0,
            short_width,
            long_width,
            pulse_tolerance: long_width.abs_diff(short_width) / 2,
            frame_gap,
            reset_gap,
            frame_bits: None,
            max_frame_bits: 4_096,
            max_distinct_frames: 256,
            min_repeats: 1,
            short_is_one: true,
            gap_pulse: PwmGapPulse::default(),
        }
    }

    /// Reject timing, threshold, and framing settings that cannot be decoded.
    fn validate(&self) -> Result<()> {
        if !self.high_threshold.is_finite() || self.high_threshold <= 0.0 {
            return Err(Error::msg(
                "PwmDecoder high threshold must be finite and greater than zero",
            ));
        }
        if !self.low_threshold.is_finite()
            || self.low_threshold <= 0.0
            || self.low_threshold > self.high_threshold
        {
            return Err(Error::msg(
                "PwmDecoder low threshold must be finite, greater than zero, and no greater than the high threshold",
            ));
        }
        if self.short_width == 0 {
            return Err(Error::msg(
                "PwmDecoder short pulse width must be greater than zero",
            ));
        }
        if self.long_width <= self.short_width {
            return Err(Error::msg(
                "PwmDecoder long pulse width must be greater than the short pulse width",
            ));
        }
        if self.frame_gap == 0 {
            return Err(Error::msg("PwmDecoder frame gap must be greater than zero"));
        }
        if self.frame_gap == usize::MAX {
            return Err(Error::msg(
                "PwmDecoder frame gap must be possible to exceed",
            ));
        }
        if self.reset_gap < self.frame_gap {
            return Err(Error::msg(
                "PwmDecoder reset gap must be at least as long as the frame gap",
            ));
        }
        if self.reset_gap == usize::MAX {
            return Err(Error::msg(
                "PwmDecoder reset gap must be possible to exceed",
            ));
        }
        if self.frame_bits == Some(0) {
            return Err(Error::msg(
                "PwmDecoder fixed frame length must be greater than zero",
            ));
        }
        if self.max_frame_bits == 0 {
            return Err(Error::msg(
                "PwmDecoder maximum frame length must be greater than zero",
            ));
        }
        if self
            .frame_bits
            .is_some_and(|frame_bits| frame_bits > self.max_frame_bits)
        {
            return Err(Error::msg(
                "PwmDecoder fixed frame length exceeds the maximum frame length",
            ));
        }
        if self.max_distinct_frames == 0 {
            return Err(Error::msg(
                "PwmDecoder maximum distinct frames must be greater than zero",
            ));
        }
        if self.min_repeats == 0 {
            return Err(Error::msg(
                "PwmDecoder minimum repeats must be greater than zero",
            ));
        }
        Ok(())
    }
}

/// Builder for [`PwmDecoder`].
#[derive(Clone, Debug)]
pub struct PwmDecoderBuilder {
    /// Settings accumulated until [`PwmDecoderBuilder::build`] is called.
    settings: PwmDecoderSettings,
}

impl PwmDecoderBuilder {
    /// Initialize a builder from the required protocol parameters.
    fn new(
        high_threshold: Float,
        short_width: usize,
        long_width: usize,
        frame_gap: usize,
        reset_gap: usize,
    ) -> Self {
        Self {
            settings: PwmDecoderSettings::new(
                high_threshold,
                short_width,
                long_width,
                frame_gap,
                reset_gap,
            ),
        }
    }

    /// Set the lower threshold used while the signal is already high.
    #[must_use]
    pub fn low_threshold(mut self, threshold: Float) -> Self {
        self.settings.low_threshold = threshold;
        self
    }

    /// Set the maximum distance from a nominal pulse width, in samples.
    #[must_use]
    pub fn pulse_tolerance(mut self, tolerance: usize) -> Self {
        self.settings.pulse_tolerance = tolerance;
        self
    }

    /// Set the required frame length, or `None` for gap-delimited frames.
    #[must_use]
    pub fn frame_bits(mut self, bits: Option<usize>) -> Self {
        self.settings.frame_bits = bits;
        self
    }

    /// Set the maximum variable frame length accepted by the decoder.
    ///
    /// This also bounds memory use for malformed or continuously pulsing input.
    #[must_use]
    pub fn max_frame_bits(mut self, bits: usize) -> Self {
        self.settings.max_frame_bits = bits;
        self
    }

    /// Set the maximum number of distinct frames tracked per transmission.
    ///
    /// Additional new bit patterns are ignored until the reset gap, bounding
    /// memory use if noise produces frame gaps but never a transmission reset.
    #[must_use]
    pub fn max_distinct_frames(mut self, frames: usize) -> Self {
        self.settings.max_distinct_frames = frames;
        self
    }

    /// Set the number of identical frames required within one transmission.
    ///
    /// A transmission ends at `reset_gap` samples of low input. Frames that do
    /// not reach this count are discarded at that point.
    #[must_use]
    pub fn min_repeats(mut self, repeats: usize) -> Self {
        self.settings.min_repeats = repeats;
        self
    }

    /// Choose whether a nominal short pulse represents one or zero.
    #[must_use]
    pub fn short_is_one(mut self, short_is_one: bool) -> Self {
        self.settings.short_is_one = short_is_one;
        self
    }

    /// Choose whether the pulse immediately before a frame gap is data.
    #[must_use]
    pub fn gap_pulse(mut self, gap_pulse: PwmGapPulse) -> Self {
        self.settings.gap_pulse = gap_pulse;
        self
    }

    /// Validate the settings and build a decoder connected to `src`.
    ///
    /// # Errors
    ///
    /// Returns an error if thresholds, timings, size limits, or repeat counts
    /// cannot form a valid decoder configuration.
    pub fn build(self, src: ReadStream<Float>) -> Result<(PwmDecoder, NCReadStream<PwmFrame>)> {
        self.settings.validate()?;
        Ok(PwmDecoder::from_settings(src, self.settings))
    }
}

/// A decoded PWM frame.
///
/// Bits are stored in transmission order, with each byte equal to zero or one.
#[derive(Clone, Debug)]
pub struct PwmFrame {
    bits: Vec<u8>,
    repeats: usize,
    first_sample: u64,
    carrier_on_power: Float,
    carrier_off_power: Float,
}

impl PartialEq for PwmFrame {
    fn eq(&self, other: &Self) -> bool {
        self.bits == other.bits
            && self.repeats == other.repeats
            && self.first_sample == other.first_sample
            && self.carrier_on_power.to_bits() == other.carrier_on_power.to_bits()
            && self.carrier_off_power.to_bits() == other.carrier_off_power.to_bits()
    }
}

impl Eq for PwmFrame {}

impl PwmFrame {
    /// Return the decoded bits in transmission order.
    #[must_use]
    pub fn bits(&self) -> &[u8] {
        &self.bits
    }

    /// Consume the frame and return its decoded bits.
    #[must_use]
    pub fn into_bits(self) -> Vec<u8> {
        self.bits
    }

    /// Return the number of identical frames seen in this transmission.
    #[must_use]
    pub fn repeats(&self) -> usize {
        self.repeats
    }

    /// Return the input-sample position where the first copy began.
    #[must_use]
    pub fn first_sample(&self) -> u64 {
        self.first_sample
    }

    /// Return mean input power while accepted frame pulses were carrier-on.
    ///
    /// The mean is weighted by sample count across all identical repeats in
    /// this decoded transmission. Pulses discarded as delimiters are not
    /// included.
    #[must_use]
    pub fn carrier_on_power(&self) -> Float {
        self.carrier_on_power
    }

    /// Return mean input power while accepted frames were carrier-off.
    ///
    /// This includes the low portions of PWM symbols and their terminating
    /// frame gaps, weighted by sample count across identical repeats.
    #[must_use]
    pub fn carrier_off_power(&self) -> Float {
        self.carrier_off_power
    }

    /// Return the carrier-on to carrier-off power ratio in decibels.
    ///
    /// The calculation is
    /// `10 * log10(carrier_on_power / carrier_off_power)`. Carrier-on power
    /// includes noise, matching the usual OOK level-ratio convention rather
    /// than subtracting carrier-off power first. `None` is returned if either
    /// mean is zero or non-finite, or if the result is non-finite.
    #[must_use]
    pub fn snr_db(&self) -> Option<Float> {
        let on = self.carrier_on_power;
        let off = self.carrier_off_power;
        if !on.is_finite() || !off.is_finite() || on <= 0.0 || off <= 0.0 {
            return None;
        }
        let snr = 10.0 * (on / off).log10();
        snr.is_finite().then_some(snr)
    }

    /// Return the frame length in bits.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bits.len()
    }

    /// Return whether the frame contains no bits.
    ///
    /// The decoder never emits empty frames, so this is always false for its
    /// output.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }
}

/// Sample-weighted power accumulated for one signal state.
#[derive(Clone, Copy, Debug, Default)]
struct PowerAccumulator {
    sum: f64,
    samples: u64,
}

impl PowerAccumulator {
    /// Add one input power sample.
    fn add(&mut self, power: Float) {
        self.sum += f64::from(power);
        self.samples = self.samples.saturating_add(1);
    }

    /// Merge another sample-weighted accumulator.
    fn merge(&mut self, other: Self) {
        self.sum += other.sum;
        self.samples = self.samples.saturating_add(other.samples);
    }

    /// Return the arithmetic mean, or NaN if there were no samples.
    fn mean(self) -> Float {
        if self.samples == 0 {
            Float::NAN
        } else {
            (self.sum / self.samples as f64) as Float
        }
    }
}

/// Carrier-on and carrier-off power accumulated for one frame or candidate.
#[derive(Clone, Copy, Debug, Default)]
struct FramePower {
    on: PowerAccumulator,
    off: PowerAccumulator,
}

impl FramePower {
    /// Merge another frame's measurements into this candidate.
    fn merge(&mut self, other: Self) {
        self.on.merge(other.on);
        self.off.merge(other.off);
    }
}

/// A completed high run waiting for gap-sensitive classification.
#[derive(Debug)]
struct PendingPulse {
    width: usize,
    power: PowerAccumulator,
}

/// Until the transmission end (`reset_gap`), candidates are kept track of. At
/// reset, the ones that reach at least `repeats` repetitions will be considered
/// valid, and will be sent at transmission end.
#[derive(Debug)]
struct Candidate {
    bits: Vec<u8>,
    repeats: usize,
    first_sample: u64,
    power: FramePower,
}

/// State for the decoding process.
#[derive(Debug)]
struct PwmState {
    /// Validated thresholds, timings, and framing behavior.
    settings: PwmDecoderSettings,

    /// Absolute input position assigned to the next sample.
    sample_index: u64,

    /// Whether the hysteresis slicer currently considers the signal high.
    high: bool,

    /// Number of consecutive samples in the current high run.
    high_run: usize,

    /// Number of consecutive samples in the current low run.
    low_run: usize,

    /// Width of the last high pulse, awaiting gap-sensitive classification.
    pending_pulse: Option<PendingPulse>,

    /// Carrier-on power accumulated for the current high run.
    high_power: PowerAccumulator,

    /// Accepted carrier-on and carrier-off power for the current frame.
    frame_power: FramePower,

    /// Bits decoded for the frame currently being assembled.
    bits: Vec<u8>,

    /// Input position where the current frame's first pulse began.
    frame_start: Option<u64>,

    /// Whether to discard pulses until the current frame gap completes.
    frame_invalid: bool,

    /// Distinct completed frames accumulated during the current transmission.
    candidates: Vec<Candidate>,
}

impl PwmState {
    /// Initialize the pulse decoder state from validated settings.
    fn new(settings: PwmDecoderSettings) -> Self {
        let capacity = settings
            .frame_bits
            .unwrap_or(64)
            .min(settings.max_frame_bits);
        Self {
            settings,
            sample_index: 0,
            high: false,
            high_run: 0,
            low_run: 0,
            pending_pulse: None,
            high_power: PowerAccumulator::default(),
            frame_power: FramePower::default(),
            bits: Vec::with_capacity(capacity),
            frame_start: None,
            frame_invalid: false,
            candidates: Vec::new(),
        }
    }

    /// Process one power sample and return frames completed by a reset gap.
    fn process(&mut self, power: Float) -> Option<Vec<PwmFrame>> {
        let output = if self.high {
            if power >= self.settings.low_threshold {
                self.high_run = self.high_run.saturating_add(1);
                self.high_power.add(power);
            } else {
                self.high = false;
                self.pending_pulse = Some(PendingPulse {
                    width: self.high_run,
                    power: std::mem::take(&mut self.high_power),
                });
                self.high_run = 0;
                self.low_run = 1;
                if self.frame_start.is_some() && !self.frame_invalid {
                    self.frame_power.off.add(power);
                }
            }
            None
        } else if power <= self.settings.high_threshold {
            self.low_run = self.low_run.saturating_add(1);
            if self.frame_start.is_some() && !self.frame_invalid {
                self.frame_power.off.add(power);
            }
            if self.low_run == self.settings.frame_gap + 1 {
                match self.settings.gap_pulse {
                    PwmGapPulse::Data => self.finish_pending_pulse(),
                    PwmGapPulse::Delimiter => self.pending_pulse = None,
                }
                self.finish_frame();
            }
            if self.low_run == self.settings.reset_gap + 1 {
                Some(self.finish_transmission())
            } else {
                None
            }
        } else {
            // Rising edge.
            if self.low_run <= self.settings.frame_gap {
                self.finish_pending_pulse();
            } else {
                self.pending_pulse = None;
            }
            self.high = true;
            self.high_run = 1;
            self.high_power = PowerAccumulator::default();
            self.high_power.add(power);
            self.low_run = 0;
            if self.bits.is_empty() && !self.frame_invalid {
                self.frame_start = Some(self.sample_index);
            }
            None
        };
        self.sample_index = self.sample_index.saturating_add(1);
        output
    }

    /// Classify and append the pulse waiting behind the current low run.
    fn finish_pending_pulse(&mut self) {
        let Some(pulse) = self.pending_pulse.take() else {
            return;
        };
        let short_distance = pulse.width.abs_diff(self.settings.short_width);
        let long_distance = pulse.width.abs_diff(self.settings.long_width);
        let short_valid = short_distance <= self.settings.pulse_tolerance;
        let long_valid = long_distance <= self.settings.pulse_tolerance;
        let short = match (short_valid, long_valid) {
            (true, true) => short_distance <= long_distance,
            (true, false) => true,
            (false, true) => false,
            (false, false) => {
                self.invalidate_frame();
                return;
            }
        };
        if self.bits.len() == self.settings.max_frame_bits {
            self.invalidate_frame();
            return;
        }
        self.bits
            .push(u8::from(short == self.settings.short_is_one));
        self.frame_power.on.merge(pulse.power);
    }

    /// Discard the current frame and ignore pulses until its gap completes.
    fn invalidate_frame(&mut self) {
        self.bits.clear();
        self.frame_start = None;
        self.frame_invalid = true;
        self.frame_power = FramePower::default();
    }

    /// Reset state associated with the current frame.
    fn clear_frame(&mut self) {
        self.bits.clear();
        self.frame_start = None;
        self.frame_invalid = false;
        self.pending_pulse = None;
        self.high_power = PowerAccumulator::default();
        self.frame_power = FramePower::default();
    }

    /// Validate and add the completed frame to the transmission candidates.
    fn finish_frame(&mut self) {
        if self.frame_invalid
            || self.bits.is_empty()
            || self
                .settings
                .frame_bits
                .is_some_and(|frame_bits| frame_bits != self.bits.len())
        {
            self.clear_frame();
            return;
        }

        let bits = std::mem::take(&mut self.bits);
        let first_sample = self.frame_start.take().unwrap_or(self.sample_index);
        let power = std::mem::take(&mut self.frame_power);
        if let Some(candidate) = self
            .candidates
            .iter_mut()
            .find(|candidate| candidate.bits == bits)
        {
            candidate.repeats = candidate.repeats.saturating_add(1);
            candidate.power.merge(power);
        } else if self.candidates.len() < self.settings.max_distinct_frames {
            self.candidates.push(Candidate {
                bits,
                repeats: 1,
                first_sample,
                power,
            });
        }
        self.clear_frame();
    }

    /// Emit repeated candidates and reset transmission-level state.
    fn finish_transmission(&mut self) -> Vec<PwmFrame> {
        self.clear_frame();
        std::mem::take(&mut self.candidates)
            .into_iter()
            .filter(|candidate| candidate.repeats >= self.settings.min_repeats)
            .map(|candidate| PwmFrame {
                bits: candidate.bits,
                repeats: candidate.repeats,
                first_sample: candidate.first_sample,
                carrier_on_power: candidate.power.on.mean(),
                carrier_off_power: candidate.power.off.mean(),
            })
            .collect()
    }
}

/// Threshold and decode a stream of PWM-modulated OOK power samples.
///
/// The output is a no-copy stream because frames own their bit vectors. Input
/// tags are intentionally not propagated; [`PwmFrame::first_sample`] records
/// each frame's absolute position in the input stream. Each frame with a
/// finite SNR also receives a position-zero [`TAG_SNR_DB`] output tag.
///
/// Output is only emitted once a transmission is ended by seeing
/// a low gap longer than the configured `reset_gap` in order to count all repeats,
/// introducing a slight delay.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct PwmDecoder {
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: NCWriteStream<PwmFrame>,
    state: PwmState,
    pending: VecDeque<PwmFrame>,
    eof_finalized: bool,
}

impl PwmDecoder {
    /// Create a PWM decoder builder.
    ///
    /// Required protocol parameters:
    ///
    /// - `high_threshold`: Power level at which a low signal becomes high. The
    ///   input is expected to be a non-negative power stream, such as the
    ///   output of `ComplexToMag2`.
    /// - `short_width`: Nominal duration, in input samples, of the shorter high
    ///   pulse used to encode one of the two bit values.
    /// - `long_width`: Nominal duration, in input samples, of the longer high
    ///   pulse. It must be greater than `short_width`.
    /// - `frame_gap`: Maximum in-frame low gap, in samples. A longer gap marks
    ///   the end of one frame. The pulse immediately before this gap is
    ///   handled according to [`PwmGapPulse`].
    /// - `reset_gap`: Maximum low gap within a transmission, in samples. A
    ///   longer gap ends the transmission and causes frame candidates
    ///   sufficiently repeated to be emitted. It must be at least as long as
    ///   `frame_gap`.
    ///
    /// Other settings start with these defaults:
    ///
    /// - The low hysteresis threshold is half of `high_threshold`.
    /// - The pulse tolerance is half the difference between the nominal pulse
    ///   widths.
    /// - `frame_bits` is `None`, so frame length is determined by `frame_gap`.
    /// - At most 4,096 bits are retained per frame.
    /// - At most 256 distinct frame patterns are tracked per transmission.
    /// - One occurrence is enough to emit a frame.
    /// - Short pulses represent one and long pulses represent zero.
    /// - The pulse before a frame gap is decoded as data.
    #[must_use]
    pub fn builder(
        high_threshold: Float,
        short_width: usize,
        long_width: usize,
        frame_gap: usize,
        reset_gap: usize,
    ) -> PwmDecoderBuilder {
        PwmDecoderBuilder::new(
            high_threshold,
            short_width,
            long_width,
            frame_gap,
            reset_gap,
        )
    }

    /// Create a decoder from validated builder settings.
    fn from_settings(
        src: ReadStream<Float>,
        settings: PwmDecoderSettings,
    ) -> (Self, NCReadStream<PwmFrame>) {
        let (dst, dst_read) = crate::stream::new_nocopy_stream();
        (
            Self {
                src,
                dst,
                state: PwmState::new(settings),
                pending: VecDeque::new(),
                eof_finalized: false,
            },
            dst_read,
        )
    }

    /// Move queued frames into the output stream while capacity is available.
    fn write_pending(&mut self) {
        while self.dst.remaining() > 0 {
            let Some(frame) = self.pending.pop_front() else {
                break;
            };
            let tags = frame
                .snr_db()
                .map(|snr| vec![Tag::new(0, TAG_SNR_DB, TagValue::Float(snr))])
                .unwrap_or_default();
            self.dst.push(frame, tags);
        }
    }

    /// Flush completed transmission candidates once when the input closes.
    fn finalize_eof(&mut self) {
        if !self.eof_finalized {
            self.pending.extend(self.state.finish_transmission());
            self.eof_finalized = true;
        }
    }
}

impl BlockEOF for PwmDecoder {
    /// Report EOF only after all buffered frames have been handed downstream.
    fn eof(&mut self) -> bool {
        self.eof_finalized && self.pending.is_empty()
    }
}

impl Block for PwmDecoder {
    /// Decode available samples and honor input and output backpressure.
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            self.write_pending();
            if !self.pending.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            if self.eof_finalized {
                return Ok(BlockRet::EOF);
            }

            let (input, _tags) = self.src.read_buf()?;
            if input.is_empty() {
                drop(input);
                if self.src.eof() {
                    self.finalize_eof();
                    self.write_pending();
                    return if self.pending.is_empty() {
                        Ok(BlockRet::EOF)
                    } else {
                        Ok(BlockRet::WaitForStream(&self.dst, 1))
                    };
                }
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }

            for &power in input.iter() {
                if let Some(frames) = self.state.process(power) {
                    self.pending.extend(frames);
                }
            }
            let consumed = input.len();
            input.consume(consumed);

            if self.src.eof() {
                self.finalize_eof();
            }
            self.write_pending();
            if !self.pending.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            } else if self.eof_finalized {
                return Ok(BlockRet::EOF);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{Tag, TagValue};
    use crate::vector_source::VectorSource;

    const SHORT: usize = 2;
    const LONG: usize = 5;
    const FRAME_GAP: usize = 9;
    const RESET_GAP: usize = 20;

    /// Build the compact decoder builder used by the synthetic tests.
    fn builder(frame_bits: Option<usize>) -> PwmDecoderBuilder {
        PwmDecoder::builder(0.5, SHORT, LONG, FRAME_GAP, RESET_GAP)
            .low_threshold(0.25)
            .pulse_tolerance(1)
            .frame_bits(frame_bits)
            .max_frame_bits(16)
    }

    /// Append a constant high or low run to a synthetic power stream.
    fn add_run(samples: &mut Vec<Float>, high: bool, count: usize) {
        samples.extend(std::iter::repeat_n(if high { 1.0 } else { 0.0 }, count));
    }

    /// Append a run at an explicit power level.
    fn add_power_run(samples: &mut Vec<Float>, power: Float, count: usize) {
        samples.extend(std::iter::repeat_n(power, count));
    }

    /// Encode one synthetic PWM frame into power samples.
    fn add_frame(samples: &mut Vec<Float>, bits: &[u8], gap_pulse: PwmGapPulse) {
        add_power_frame(samples, bits, gap_pulse, 1.0, 0.0, 1.0);
    }

    /// Encode one synthetic PWM frame with explicit on, off, and delimiter power.
    fn add_power_frame(
        samples: &mut Vec<Float>,
        bits: &[u8],
        gap_pulse: PwmGapPulse,
        on_power: Float,
        off_power: Float,
        delimiter_power: Float,
    ) {
        for &bit in bits {
            let short = bit == 1;
            add_power_run(samples, on_power, if short { SHORT } else { LONG });
            add_power_run(samples, off_power, if short { LONG } else { SHORT });
        }
        if gap_pulse == PwmGapPulse::Delimiter {
            add_power_run(samples, delimiter_power, SHORT);
        }
        add_power_run(samples, off_power, FRAME_GAP + 1);
    }

    /// Run the decoder to EOF and collect all emitted frames.
    fn decode(samples: &[Float], builder: PwmDecoderBuilder) -> Result<Vec<PwmFrame>> {
        let outputs = decode_with_tags(samples, builder)?;
        let mut frames = Vec::new();
        for (frame, tags) in outputs {
            assert!(tags.is_empty());
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Run the decoder to EOF and retain generated output tags.
    fn decode_with_tags(
        samples: &[Float],
        builder: PwmDecoderBuilder,
    ) -> Result<Vec<(PwmFrame, Vec<Tag>)>> {
        let src = ReadStream::from_slice(samples);
        let (mut decoder, output) = builder.build(src)?;
        loop {
            if matches!(decoder.work()?, BlockRet::EOF) {
                break;
            }
        }
        let mut outputs = Vec::new();
        while let Some(output) = output.pop() {
            outputs.push(output);
        }
        Ok(outputs)
    }

    /// Verify fixed-length frames are grouped by their repeated bit pattern.
    #[test]
    fn fixed_frames_are_matched_and_repeated() -> Result<()> {
        let target = [1, 0, 1, 1, 0];
        let mut samples = Vec::new();
        add_run(&mut samples, false, RESET_GAP);
        add_frame(&mut samples, &[1, 1, 1, 1, 1], PwmGapPulse::Delimiter);
        for _ in 0..3 {
            add_frame(&mut samples, &target, PwmGapPulse::Delimiter);
        }
        add_run(&mut samples, false, RESET_GAP);

        let frames = decode(
            &samples,
            builder(Some(target.len()))
                .gap_pulse(PwmGapPulse::Delimiter)
                .min_repeats(3),
        )?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bits(), target);
        assert_eq!(frames[0].repeats(), 3);
        assert!(!frames[0].is_empty());
        Ok(())
    }

    /// Verify a frame gap can terminate frames of different lengths.
    #[test]
    fn variable_frames_end_at_the_frame_gap() -> Result<()> {
        let first = [1, 0, 1];
        let second = [0, 1, 1, 0, 0];
        let mut samples = Vec::new();
        add_frame(&mut samples, &first, PwmGapPulse::Data);
        add_frame(&mut samples, &second, PwmGapPulse::Data);
        add_run(&mut samples, false, RESET_GAP);

        let frames = decode(&samples, builder(None))?;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].bits(), first);
        assert_eq!(frames[1].bits(), second);
        Ok(())
    }

    /// Verify fixed-length mode discards frames with the wrong size.
    #[test]
    fn fixed_length_rejects_other_lengths() -> Result<()> {
        let mut samples = Vec::new();
        add_frame(&mut samples, &[1, 0, 1], PwmGapPulse::Data);
        add_run(&mut samples, false, RESET_GAP);
        assert!(decode(&samples, builder(Some(4)))?.is_empty());
        Ok(())
    }

    /// Verify the pulse before a gap can be treated as data or a delimiter.
    #[test]
    fn gap_pulse_can_be_data_or_a_delimiter() -> Result<()> {
        let mut samples = Vec::new();
        add_frame(&mut samples, &[1, 0, 1], PwmGapPulse::Delimiter);
        add_run(&mut samples, false, RESET_GAP);

        let delimiter = decode(&samples, builder(None).gap_pulse(PwmGapPulse::Delimiter))?;
        let data = decode(&samples, builder(None).gap_pulse(PwmGapPulse::Data))?;
        assert_eq!(delimiter[0].bits(), &[1, 0, 1]);
        assert_eq!(data[0].bits(), &[1, 0, 1, 1]);
        Ok(())
    }

    /// Verify accepted repeats produce sample-weighted power and SNR metadata.
    #[test]
    fn reports_power_and_snr_for_accepted_repeats() -> Result<()> {
        fn require_eq<T: Eq>() {}

        let target = [1, 0, 1];
        let mut samples = Vec::new();
        // This wrong-length frame must not affect the accepted candidate.
        add_power_frame(
            &mut samples,
            &[1, 0],
            PwmGapPulse::Delimiter,
            100.0,
            0.2,
            200.0,
        );
        add_power_frame(
            &mut samples,
            &target,
            PwmGapPulse::Delimiter,
            1.0,
            0.1,
            100.0,
        );
        add_power_frame(
            &mut samples,
            &target,
            PwmGapPulse::Delimiter,
            3.0,
            0.1,
            100.0,
        );
        add_run(&mut samples, false, RESET_GAP);

        let outputs = decode_with_tags(
            &samples,
            builder(Some(target.len()))
                .gap_pulse(PwmGapPulse::Delimiter)
                .min_repeats(2),
        )?;
        assert_eq!(outputs.len(), 1);
        let (frame, tags) = &outputs[0];
        assert_eq!(frame.bits(), target);
        assert_eq!(frame.repeats(), 2);
        assert!((frame.carrier_on_power() - 2.0).abs() < 1e-6);
        assert!((frame.carrier_off_power() - 0.1).abs() < 1e-6);
        let snr_db = frame.snr_db().expect("positive finite power levels");
        assert!((snr_db - 10.0 * 20.0_f32.log10()).abs() < 1e-5);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].pos(), 0);
        assert_eq!(tags[0].key(), TAG_SNR_DB);
        assert_eq!(tags[0].val(), &TagValue::Float(snr_db));

        require_eq::<PwmFrame>();
        Ok(())
    }

    /// Verify invalid power levels do not produce non-finite SNR metadata.
    #[test]
    fn unavailable_snr_is_not_tagged() -> Result<()> {
        let mut samples = Vec::new();
        add_frame(&mut samples, &[1, 0, 1], PwmGapPulse::Data);
        add_run(&mut samples, false, RESET_GAP);
        let outputs = decode_with_tags(&samples, builder(None))?;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.carrier_on_power(), 1.0);
        assert_eq!(outputs[0].0.carrier_off_power(), 0.0);
        assert_eq!(outputs[0].0.snr_db(), None);
        assert!(outputs[0].1.is_empty());

        let mut frame = outputs.into_iter().next().expect("one output").0;
        frame.carrier_off_power = Float::NAN;
        assert_eq!(frame.snr_db(), None);
        Ok(())
    }

    /// Verify gaps must exceed, rather than equal, their configured thresholds.
    #[test]
    fn gap_thresholds_are_strict() {
        let mut state = PwmState::new(builder(Some(2)).settings);

        for _ in 0..SHORT {
            assert!(state.process(1.0).is_none());
        }
        for _ in 0..FRAME_GAP {
            assert!(state.process(0.0).is_none());
        }
        // A rising edge after exactly frame_gap low samples continues the row.
        for _ in 0..LONG {
            assert!(state.process(1.0).is_none());
        }
        for _ in 0..RESET_GAP {
            assert!(state.process(0.0).is_none());
        }

        let frames = state
            .process(0.0)
            .expect("reset threshold should be crossed");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bits(), &[1, 0]);
    }

    /// Verify short pulses can represent either binary value.
    #[test]
    fn pulse_polarity_can_be_inverted() -> Result<()> {
        let mut samples = Vec::new();
        add_frame(&mut samples, &[1, 0, 1], PwmGapPulse::Data);
        add_run(&mut samples, false, RESET_GAP);

        let frames = decode(&samples, builder(None).short_is_one(false))?;
        assert_eq!(frames[0].bits(), &[0, 1, 0]);
        Ok(())
    }

    /// Verify distinct-frame tracking stops at the configured bound.
    #[test]
    fn distinct_frame_limit_bounds_transmission_state() -> Result<()> {
        let mut samples = Vec::new();
        add_frame(&mut samples, &[1, 0, 1], PwmGapPulse::Data);
        add_frame(&mut samples, &[0, 1, 0], PwmGapPulse::Data);
        add_run(&mut samples, false, RESET_GAP);

        let frames = decode(&samples, builder(None).max_distinct_frames(1))?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bits(), &[1, 0, 1]);
        Ok(())
    }

    /// Verify EOF flushes complete frames without accepting a partial frame.
    #[test]
    fn eof_flushes_completed_frames_but_not_a_partial_frame() -> Result<()> {
        let complete = [1, 0, 0];
        let mut samples = Vec::new();
        add_frame(&mut samples, &complete, PwmGapPulse::Data);
        // This frame never reaches a frame gap and must not be emitted at EOF.
        add_run(&mut samples, true, SHORT);
        add_run(&mut samples, false, LONG);

        let frames = decode(&samples, builder(None))?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bits(), complete);
        Ok(())
    }

    /// Verify input stream tags do not appear on decoded frames.
    #[test]
    fn input_tags_are_intentionally_dropped() -> Result<()> {
        let mut samples = Vec::new();
        add_frame(&mut samples, &[1, 0, 1], PwmGapPulse::Data);
        add_run(&mut samples, false, RESET_GAP);
        let tag = Tag::new(1, "input", TagValue::Bool(true));
        let (mut source, input) = VectorSource::builder(samples).tags(&[tag]).build()?;
        let (mut decoder, output) = builder(None).build(input)?;

        source.work()?;
        decoder.work()?;
        let (_, tags) = output.pop().expect("expected one decoded frame");
        assert!(tags.is_empty());
        Ok(())
    }

    /// Verify queued frames survive backpressure and drain before EOF.
    #[test]
    fn pending_output_obeys_backpressure_and_drains_before_eof() -> Result<()> {
        let mut samples = Vec::new();
        for value in 0_u16..1_001 {
            let bits = (0..10)
                .rev()
                .map(|shift| ((value >> shift) & 1) as u8)
                .collect::<Vec<_>>();
            add_frame(&mut samples, &bits, PwmGapPulse::Data);
            add_run(&mut samples, false, RESET_GAP - FRAME_GAP);
        }

        let src = ReadStream::from_slice(&samples);
        let (mut decoder, output) = builder(Some(10))
            .max_frame_bits(10)
            .min_repeats(1)
            .build(src)?;
        assert!(matches!(decoder.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(!BlockEOF::eof(&mut decoder));
        let first = output.pop().expect("output queue should be full").0;
        assert_eq!(first.bits(), &[0; 10]);

        assert!(matches!(decoder.work()?, BlockRet::EOF));
        assert!(BlockEOF::eof(&mut decoder));
        let mut count = 1;
        while output.pop().is_some() {
            count += 1;
        }
        assert_eq!(count, 1_001);
        Ok(())
    }

    /// Verify invalid builder settings are rejected by `build`.
    #[test]
    fn builder_validates_settings() {
        let src = || ReadStream::from_slice(&[]);
        assert!(builder(None).build(src()).is_ok());
        assert!(PwmDecoder::builder(0.0, 2, 5, 9, 20).build(src()).is_err());
        assert!(
            PwmDecoder::builder(0.5, 2, 5, 9, 20)
                .low_threshold(Float::NAN)
                .build(src())
                .is_err()
        );
        assert!(PwmDecoder::builder(0.5, 0, 5, 9, 20).build(src()).is_err());
        assert!(PwmDecoder::builder(0.5, 5, 5, 9, 20).build(src()).is_err());
        assert!(PwmDecoder::builder(0.5, 2, 5, 0, 20).build(src()).is_err());
        assert!(PwmDecoder::builder(0.5, 2, 5, 9, 8).build(src()).is_err());
        assert!(
            PwmDecoder::builder(0.5, 2, 5, usize::MAX, usize::MAX)
                .build(src())
                .is_err()
        );
        assert!(
            PwmDecoder::builder(0.5, 2, 5, 9, usize::MAX)
                .build(src())
                .is_err()
        );
        assert!(builder(None).frame_bits(Some(0)).build(src()).is_err());
        assert!(builder(None).max_frame_bits(0).build(src()).is_err());
        assert!(builder(None).frame_bits(Some(17)).build(src()).is_err());
        assert!(builder(None).max_distinct_frames(0).build(src()).is_err());
        assert!(builder(None).min_repeats(0).build(src()).is_err());
    }
}
