//! UHD complex-sample sink.

use std::time::Duration;

use log::{debug, warn};

use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;
use crate::uhd_device::UhdDevice;
use crate::{Complex, Error, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10);

/// Builder for [`UhdSink`].
#[derive(Debug)]
#[must_use]
pub struct UhdSinkBuilder {
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

impl UhdSinkBuilder {
    /// Select the UHD transmit channel. The default is channel 0.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }

    /// Set transmit gain in dB. By default the device's current gain is kept.
    pub fn gain(mut self, gain: f64) -> Self {
        self.gain = Some(gain);
        self
    }

    /// Select a named gain element. An empty name selects UHD's overall gain.
    pub fn gain_element(mut self, name: impl Into<String>) -> Self {
        self.gain_element = name.into();
        self
    }

    /// Select the transmit antenna. By default the current antenna is kept.
    pub fn antenna(mut self, antenna: impl Into<String>) -> Self {
        self.antenna = Some(antenna.into());
        self
    }

    /// Set transmit frontend bandwidth in Hz.
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

    /// Set the maximum time spent in one UHD transmit call.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Configure the device and build a sink consuming `src`.
    pub fn build(self, src: ReadStream<Complex>) -> Result<UhdSink> {
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
            return Err(Error::msg("UHD transmit timeout must be greater than zero"));
        }

        {
            let mut usrp = self.device.lock()?;
            let channels = usrp.get_num_tx_channels()?;
            if self.channel >= channels {
                return Err(Error::msg(format!(
                    "UHD transmit channel {} is out of range; device has {channels} channels",
                    self.channel
                )));
            }

            usrp.set_tx_sample_rate(self.sample_rate, self.channel)?;
            let tune = usrp.set_tx_frequency(
                &uhd::TuneRequest::with_frequency(self.frequency),
                self.channel,
            )?;
            debug!("UHD TX tune result: {tune:?}");
            if let Some(gain) = self.gain {
                usrp.set_tx_gain(gain, self.channel, &self.gain_element)?;
            }
            if let Some(bandwidth) = self.bandwidth {
                usrp.set_tx_bandwidth(bandwidth, self.channel)?;
            }
            if let Some(antenna) = &self.antenna {
                usrp.set_tx_antenna(antenna, self.channel)?;
            }

            debug!(
                "UHD TX channel={} frequency={} sample_rate={} gain={} bandwidth={} antenna={}",
                self.channel,
                usrp.get_tx_frequency(self.channel)?,
                usrp.get_tx_sample_rate(self.channel)?,
                usrp.get_tx_gain(self.channel, &self.gain_element)?,
                usrp.get_tx_bandwidth(self.channel)?,
                usrp.get_tx_antenna(self.channel)?,
            );
        }

        let stream_args = uhd::StreamArgs::<Complex>::builder()
            .wire_format(self.wire_format)
            .args(self.stream_args)
            .channels(vec![self.channel])
            .build();
        let stream = self.device.tx_stream(&stream_args)?;
        Ok(UhdSink {
            // Keep this field before `_device`; stream must be dropped first.
            stream,
            _device: self.device,
            src,
            timeout: self.timeout.as_secs_f64(),
            start_of_burst: true,
            ended: false,
        })
    }
}

/// Transmit I/Q samples through a UHD device.
///
/// The sink marks the first accepted samples as the start of a burst and sends
/// an end-of-burst marker when its input reaches EOF or the block is dropped.
/// Input stream tags are currently ignored; timed and explicitly segmented
/// bursts are therefore not inferred from tags.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct UhdSink {
    stream: uhd::TransmitStreamer<'static, Complex>,
    _device: UhdDevice,

    #[rustradio(in)]
    src: ReadStream<Complex>,

    timeout: f64,
    start_of_burst: bool,
    ended: bool,
}

impl UhdSink {
    /// Create a builder using `device`, center `frequency`, and `sample_rate`.
    pub fn builder(device: &UhdDevice, frequency: f64, sample_rate: f64) -> UhdSinkBuilder {
        UhdSinkBuilder {
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

    fn end_burst(&mut self) -> Result<()> {
        if self.ended {
            return Ok(());
        }
        let empty: &[Complex] = &[];
        let mut metadata = uhd::TransmitMetadata::with_flags(false, true, None);
        self.stream
            .send(&mut [empty], &mut metadata, self.timeout)?;
        self.ended = true;
        Ok(())
    }
}

impl Block for UhdSink {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            let (input, _tags) = self.src.read_buf()?;
            if input.is_empty() {
                if self.src.eof() {
                    self.end_burst()?;
                    return Ok(BlockRet::EOF);
                }
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }

            let input_len = input.len();
            let mut metadata = uhd::TransmitMetadata::with_flags(self.start_of_burst, false, None);
            let sent = self
                .stream
                .send(&mut [input.slice()], &mut metadata, self.timeout)?;
            if sent == 0 {
                return Ok(BlockRet::Pending);
            }

            self.start_of_burst = false;
            input.consume(sent);
            if sent == input_len {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
        }
    }
}

impl Drop for UhdSink {
    fn drop(&mut self) {
        if let Err(error) = self.end_burst() {
            warn!("failed to end UHD transmit burst: {error}");
        }
    }
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
    fn rejects_invalid_numeric_configuration() {
        assert!(validate_positive_finite("frequency", -1.0).is_err());
        assert!(validate_positive_finite("sample rate", f64::INFINITY).is_err());
        assert!(validate_finite("gain", f64::NAN).is_err());
        assert!(validate_positive_finite("bandwidth", 1.0).is_ok());
    }
}
