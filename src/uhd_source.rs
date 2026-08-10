//! UHD complex-sample source.

use std::time::Duration;

use log::{debug, trace, warn};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};
use crate::uhd_device::UhdDevice;
use crate::{Complex, Error, Float, Result};

const TAG_PREFIX: &str = "UhdSource::";
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10);

/// Builder for [`UhdSource`].
#[derive(Debug)]
#[must_use]
pub struct UhdSourceBuilder {
    device: UhdDevice,
    frequency: f64,
    sample_rate: f64,
    channel: usize,
    gain: Option<f64>,
    gain_element: String,
    antenna: Option<String>,
    bandwidth: Option<f64>,
    wire_format: String,
    stream_args: String,
    timeout: Duration,
}

impl UhdSourceBuilder {
    /// Select the UHD receive channel. The default is channel 0.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }

    /// Set receive gain in dB. By default the device's current gain is kept.
    pub fn gain(mut self, gain: f64) -> Self {
        self.gain = Some(gain);
        self
    }

    /// Select a named gain element. An empty name selects UHD's overall gain.
    pub fn gain_element(mut self, name: impl Into<String>) -> Self {
        self.gain_element = name.into();
        self
    }

    /// Select the receive antenna. By default the current antenna is kept.
    pub fn antenna(mut self, antenna: impl Into<String>) -> Self {
        self.antenna = Some(antenna.into());
        self
    }

    /// Set receive frontend bandwidth in Hz.
    pub fn bandwidth(mut self, bandwidth: f64) -> Self {
        self.bandwidth = Some(bandwidth);
        self
    }

    /// Set the UHD over-the-wire sample format. The default is `sc16`.
    pub fn wire_format(mut self, wire_format: impl Into<String>) -> Self {
        self.wire_format = wire_format.into();
        self
    }

    /// Set device-specific UHD stream arguments.
    pub fn stream_args(mut self, stream_args: impl Into<String>) -> Self {
        self.stream_args = stream_args.into();
        self
    }

    /// Set the maximum time spent in one UHD receive call.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Configure the device and build the source and its output stream.
    pub fn build(self) -> Result<(UhdSource, ReadStream<Complex>)> {
        validate_positive_finite("frequency", self.frequency)?;
        validate_positive_finite("sample rate", self.sample_rate)?;
        if let Some(gain) = self.gain {
            validate_finite("gain", gain)?;
        }
        if let Some(bandwidth) = self.bandwidth {
            validate_positive_finite("bandwidth", bandwidth)?;
        }
        if self.wire_format.is_empty() {
            return Err(Error::msg("UHD wire format must not be empty"));
        }
        if self.timeout.is_zero() {
            return Err(Error::msg("UHD receive timeout must be greater than zero"));
        }

        let mut initial_tags = {
            let mut usrp = self.device.lock()?;
            let channels = usrp.get_num_rx_channels()?;
            if self.channel >= channels {
                return Err(Error::msg(format!(
                    "UHD receive channel {} is out of range; device has {channels} channels",
                    self.channel
                )));
            }

            usrp.set_rx_sample_rate(self.sample_rate, self.channel)?;
            let tune = usrp.set_rx_frequency(
                &uhd::TuneRequest::with_frequency(self.frequency),
                self.channel,
            )?;
            debug!("UHD RX tune result: {tune:?}");
            if let Some(gain) = self.gain {
                usrp.set_rx_gain(gain, self.channel, &self.gain_element)?;
            }
            if let Some(bandwidth) = self.bandwidth {
                usrp.set_rx_bandwidth(bandwidth, self.channel)?;
            }
            if let Some(antenna) = &self.antenna {
                usrp.set_rx_antenna(antenna, self.channel)?;
            }

            initial_tags(&usrp, self.channel, &self.gain_element, &self.wire_format)?
        };

        initial_tags.push(Tag::new(
            0,
            format!("{TAG_PREFIX}stream_args"),
            TagValue::String(self.stream_args.clone()),
        ));

        let stream_args = uhd::StreamArgs::<Complex>::builder()
            .wire_format(self.wire_format)
            .args(self.stream_args)
            .channels(vec![self.channel])
            .build();
        let stream = self.device.rx_stream(&stream_args)?;
        let (dst, output) = crate::stream::new_stream();
        Ok((
            UhdSource {
                // Keep this field before `_device`; stream must be dropped first.
                stream,
                _device: self.device,
                dst,
                timeout: self.timeout.as_secs_f64(),
                pending_tags: initial_tags,
                started: false,
            },
            output,
        ))
    }
}

/// Receive I/Q samples from a UHD device.
///
/// Each UHD receive call is limited to one transport packet.
///
/// This block attaches these tags:
///
/// - `UhdSource::time_ns`
/// - `UhdSource::burst` (`true` on burst start, `false` on burst end)
/// - `UhdSource::error` (`bool`), `error_kind` and `error_message` when an
///   error is present
///
/// Metadata for a non-timeout event carrying no samples is deferred to the next
/// sample (i.e. an error can't be attached if there's no sample to attach it to
/// yet).
///
/// Timeout metadata is not emitted because it describes the absence of a
/// packet. Initial channel, tuning, device identity, and streamer settings are
/// also tagged on the first sample.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct UhdSource {
    stream: uhd::ReceiveStreamer<'static, Complex>,
    _device: UhdDevice,

    #[rustradio(out)]
    dst: WriteStream<Complex>,

    // Timeout in seconds.
    timeout: f64,

    // Tags can arrive even if there were zero samples, in an error condition.
    // Because we can't send tags without samples, they are stored here for
    // later.
    pending_tags: Vec<Tag>,
    started: bool,
}

impl UhdSource {
    /// Create a builder using `device`, center `frequency`, and `sample_rate`.
    pub fn builder(device: &UhdDevice, frequency: f64, sample_rate: f64) -> UhdSourceBuilder {
        UhdSourceBuilder {
            device: device.clone(),
            frequency,
            sample_rate,
            channel: 0,
            gain: None,
            gain_element: String::new(),
            antenna: None,
            bandwidth: None,
            wire_format: "sc16".to_string(),
            stream_args: String::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl Block for UhdSource {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            // Only start the stream once the graph is actually running, to avoid
            // warnings of underflow.
            if !self.started {
                self.stream.send_command(&uhd::StreamCommand {
                    time: uhd::StreamTime::Now,
                    command_type: uhd::StreamCommandType::StartContinuous,
                })?;
                self.started = true;
            }

            let mut output = self.dst.write_buf()?;
            if output.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }

            let metadata = self
                .stream
                .receive(&mut [&mut output.slice()], self.timeout, true)?;
            let samples = metadata.samples();
            let error = metadata.last_error();

            if samples == 0 {
                self.pending_tags.extend(metadata_tags(&metadata));
                match error.as_ref().map(uhd::ReceiveError::kind) {
                    None => {}
                    Some(uhd::ReceiveErrorKind::Timeout | uhd::ReceiveErrorKind::OutOfSequence) => {
                        trace!("UHD receive timed out");
                    }
                    Some(uhd::ReceiveErrorKind::Overflow) => {
                        warn!("UHD receive metadata error: {error:?}");
                        // Immediately retry. Returning Pending would only make
                        // things worse.
                        continue;
                    }
                    Some(_other) => warn!("UHD receive metadata error: {error:?}"),
                }
                return Ok(BlockRet::Pending);
            }
            assert!(
                samples <= output.len(),
                "got more samples than we can handle: {samples} > {}",
                output.len()
            );

            self.pending_tags.extend(metadata_tags(&metadata));
            output.produce(samples, &self.pending_tags);
            self.pending_tags.clear();
        }
    }
}

impl Drop for UhdSource {
    fn drop(&mut self) {
        if self.started
            && let Err(error) = self.stream.send_command(&uhd::StreamCommand {
                time: uhd::StreamTime::Now,
                command_type: uhd::StreamCommandType::StopContinuous,
            })
        {
            warn!("Failed to stop UHD receive stream: {error}");
        }
    }
}

fn initial_tags(
    usrp: &uhd::Usrp,
    channel: usize,
    gain_element: &str,
    wire_format: &str,
) -> Result<Vec<Tag>> {
    let info = usrp.get_rx_info(channel)?;
    let mut tags = vec![
        tag_u64("channel", channel as u64),
        tag_f64("frequency", usrp.get_rx_frequency(channel)?),
        tag_f64("sample_rate", usrp.get_rx_sample_rate(channel)?),
        tag_f64("gain", usrp.get_rx_gain(channel, gain_element)?),
        tag_f64("bandwidth", usrp.get_rx_bandwidth(channel)?),
        tag_string("antenna", usrp.get_rx_antenna(channel)?),
        tag_string("wire_format", wire_format),
        tag_string("motherboard_id", info.motherboard_id()),
        tag_string("motherboard_name", info.motherboard_name()),
        // Disable serial numbers for privacy.
        // tag_string("motherboard_serial", info.motherboard_serial()),
        // tag_string("daughterboard_serial", info.daughterboard_serial()),
        tag_string("daughterboard_id", info.daughterboard_id()),
        tag_string("subdevice_name", info.subdev_name()),
        tag_string("subdevice_spec", info.subdev_spec()),
    ];
    if let Ok(clock_source) = usrp.get_clock_source(0) {
        tags.push(tag_string("clock_source", clock_source));
    }
    if let Ok(time_source) = usrp.get_time_source(0) {
        tags.push(tag_string("time_source", time_source));
    }
    Ok(tags)
}

// Tags added to every sample read. This is just the timestamp, unless something
// unexpected happens.
fn metadata_tags(metadata: &uhd::ReceiveMetadata) -> Vec<Tag> {
    let time = metadata.time_spec();
    let error = metadata.last_error();
    let mut tags = Vec::new();
    if let Some(ref err) = error {
        warn!("UhdSource: {err:?}");
        tags.push(metadata_tag("error", TagValue::Bool(true)));
    }
    if metadata.start_of_burst() {
        tags.push(metadata_tag("burst", TagValue::Bool(true)));
    }
    if metadata.end_of_burst() {
        tags.push(metadata_tag("burst", TagValue::Bool(false)));
    }
    if metadata.more_fragments() || metadata.fragment_offset() > 0 {
        trace!(
            "Fragmented read. More: {} Offset: {}",
            metadata.more_fragments(),
            metadata.fragment_offset()
        );
    }
    if metadata.out_of_sequence() {
        tags.push(metadata_tag("out_of_sequence", TagValue::Bool(true)));
    }
    if let Some(time) = time {
        tags.push(metadata_tag(
            "time_ns",
            TagValue::I64(
                time.seconds * 1_000_000_000_i64 + (1_000_000_000_f64 * time.fraction) as i64,
            ),
        ));
    }
    if let Some(error) = error {
        tags.push(metadata_tag(
            "error_kind",
            TagValue::String(error_kind_name(error.kind()).to_string()),
        ));
        if let Some(message) = error.message() {
            tags.push(metadata_tag(
                "error_message",
                TagValue::String(message.to_string()),
            ));
        }
    }
    tags
}

fn error_kind_name(kind: uhd::ReceiveErrorKind) -> &'static str {
    match kind {
        uhd::ReceiveErrorKind::Timeout => "timeout",
        uhd::ReceiveErrorKind::LateCommand => "late_command",
        uhd::ReceiveErrorKind::BrokenChain => "broken_chain",
        uhd::ReceiveErrorKind::Overflow => "overflow",
        uhd::ReceiveErrorKind::OutOfSequence => "out_of_sequence",
        uhd::ReceiveErrorKind::Alignment => "alignment",
        uhd::ReceiveErrorKind::BadPacket => "bad_packet",
        uhd::ReceiveErrorKind::Other => "other",
        _ => "unknown",
    }
}

fn metadata_tag(name: &str, value: TagValue) -> Tag {
    Tag::new(0, format!("{TAG_PREFIX}{name}"), value)
}

fn tag_string(name: &str, value: impl Into<String>) -> Tag {
    Tag::new(
        0,
        format!("{TAG_PREFIX}{name}"),
        TagValue::String(value.into()),
    )
}

fn tag_f64(name: &str, value: f64) -> Tag {
    Tag::new(
        0,
        format!("{TAG_PREFIX}{name}"),
        TagValue::Float(value as Float),
    )
}

fn tag_u64(name: &str, value: u64) -> Tag {
    Tag::new(0, format!("{TAG_PREFIX}{name}"), TagValue::U64(value))
}

fn validate_finite(name: &str, value: f64) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(Error::msg(format!("UHD {name} must be finite")))
    }
}

fn validate_positive_finite(name: &str, value: f64) -> Result<()> {
    validate_finite(name, value)?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(Error::msg(format!("UHD {name} must be greater than zero")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_metadata_has_no_event_tags() {
        let metadata = uhd::ReceiveMetadata::new();
        let tags = metadata_tags(&metadata);
        assert!(tags.is_empty());
    }

    #[test]
    fn rejects_invalid_numeric_configuration() {
        assert!(validate_positive_finite("frequency", 0.0).is_err());
        assert!(validate_positive_finite("sample rate", f64::NAN).is_err());
        assert!(validate_finite("gain", f64::INFINITY).is_err());
        assert!(validate_positive_finite("bandwidth", 1.0).is_ok());
    }
}
