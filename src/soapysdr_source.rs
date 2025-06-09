//! SoapySDR source.
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};
use crate::{Complex, Error, Float, Result};

impl From<soapysdr::Error> for Error {
    fn from(e: soapysdr::Error) -> Self {
        Error::device(e, "soapysdr")
    }
}

/// SoapySDR source builder.
pub struct SoapySdrSourceBuilder<'a> {
    dev: &'a soapysdr::Device,
    antenna: Option<String>,
    channel: usize,
    igain: f64,
    samp_rate: f64,
    freq: f64,
}

macro_rules! log_and_tag {
    ($tags:ident, $expr:expr, $tag_key:expr) => {
        match $expr {
            Ok(s) => {
                debug!("SoapySDR RX {}: {s}", $tag_key);
                $tags.push(Tag::new(
                    0,
                    concat!("SoapySdrSource::", $tag_key),
                    TagValue::String(s),
                ));
            }
            Err(e) => debug!("SoapySDR RX {} error: {e}", $tag_key),
        }
    };
}

impl SoapySdrSourceBuilder<'_> {
    /// Set channel number.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }
    /// Set input gain.
    ///
    /// Normalized to 0.0 to 1.0.
    pub fn igain(mut self, igain: f64) -> Self {
        self.igain = igain;
        self
    }
    /// Set antenna.
    pub fn antenna<T: Into<String>>(mut self, a: T) -> Self {
        self.antenna = Some(a.into());
        self
    }
    /// Build the source object.
    pub fn build(self) -> Result<(SoapySdrSource, ReadStream<Complex>)> {
        let mut tags = vec![
            Tag::new(
                0,
                "SoapySdrSource::channel",
                TagValue::U64(self.channel as u64),
            ),
            Tag::new(
                0,
                "SoapySdrSource::frequency",
                TagValue::Float(self.freq as Float),
            ),
            Tag::new(
                0,
                "SoapySdrSource::sample_rate",
                TagValue::Float(self.samp_rate as Float),
            ),
        ];
        log_and_tag!(tags, self.dev.driver_key(), "driver");
        log_and_tag!(tags, self.dev.hardware_key(), "hardware");
        debug!("SoapySDR RX hardware info: {}", self.dev.hardware_info()?);
        log_and_tag!(
            tags,
            self.dev.frontend_mapping(soapysdr::Direction::Rx),
            "frontend_mapping"
        );
        log_and_tag!(tags, self.dev.get_clock_source(), "clock_source");
        log_and_tag!(tags, self.dev.get_time_source(), "time_source");
        // TODO: enable once
        // <https://github.com/kevinmehall/rust-soapysdr/pull/41> is merged.
        /*
        for sensor in self.dev.list_sensors()? {
            debug!(
                "SoapySDR RX sensor {sensor}: {:?}",
                self.dev.get_sensor_info(&sensor)?
            );
            debug!(
                "SoapySDR RX sensor {sensor}: {:?}",
                self.dev.read_sensor(&sensor)?
            );
        }
        */
        debug!(
            "SoapySDR RX clock sources: {:?}",
            self.dev.list_clock_sources()?
        );
        let chans = self.dev.num_channels(soapysdr::Direction::Rx)?;
        debug!("SoapySDR RX channels : {chans}");
        for channel in 0..chans {
            // TODO: enable once
            // <https://github.com/kevinmehall/rust-soapysdr/pull/41> is merged.
            /*
            for sensor in self.dev.list_channel_sensors(soapysdr::Direction::Rx, channel)? {
                match self.dev.read_channel_sensor(soapysdr::Direction::Rx, channel, &sensor) {
                    Ok(s) => debug!("SoapySDR RX channel sensor {sensor}: {s}"),
                    Err(e) => debug!("SoapySDR RX channel sensor {sensor} error: {e}"),
                }
            }
            */
            debug!(
                "SoapySDR RX channel {channel} antennas: {:?}",
                self.dev.antennas(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} gains: {:?}",
                self.dev.list_gains(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} gain range: {:?}",
                self.dev.gain_range(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} frequency range: {:?}",
                self.dev.frequency_range(soapysdr::Direction::Rx, channel)?
            );
            for ai in self
                .dev
                .stream_args_info(soapysdr::Direction::Rx, channel)?
            {
                debug!("SoapySDR RX channel {channel} arg info: {}", ai_string(&ai));
            }
            debug!(
                "SoapySDR RX channel {channel} stream formats: {:?}",
                self.dev.stream_formats(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} info: {}",
                self.dev.channel_info(soapysdr::Direction::Rx, channel)?
            );
        }
        self.dev.set_frequency(
            soapysdr::Direction::Rx,
            self.channel,
            self.freq,
            soapysdr::Args::new(),
        )?;
        self.dev
            .set_sample_rate(soapysdr::Direction::Rx, self.channel, self.samp_rate)?;
        let gr = self.dev.gain_range(soapysdr::Direction::Rx, self.channel)?;
        let gain = gr.minimum + self.igain * (gr.maximum - gr.minimum);
        self.dev
            .set_gain(soapysdr::Direction::Rx, self.channel, gain)?;
        if let Some(a) = self.antenna {
            // TODO: set antenna even if not specified.
            tags.push(Tag::new(
                0,
                "SoapySdrSource::antenna",
                TagValue::String(a.clone()),
            ));
            self.dev
                .set_antenna(soapysdr::Direction::Rx, self.channel, a)?;
        }
        let mut stream = self.dev.rx_stream(&[self.channel])?;
        stream.activate(None)?;
        let (dst, dr) = crate::stream::new_stream();
        Ok((SoapySdrSource { stream, dst, tags }, dr))
    }
}

/// SoapySDR source.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SoapySdrSource {
    stream: soapysdr::RxStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
    #[rustradio(default)]
    tags: Vec<Tag>,
}

impl SoapySdrSource {
    /// Create new SoapySdrSource builder.
    pub fn builder(dev: &soapysdr::Device, freq: f64, samp_rate: f64) -> SoapySdrSourceBuilder {
        SoapySdrSourceBuilder {
            dev,
            freq,
            samp_rate,
            channel: 0,
            igain: 0.5,
            antenna: None,
        }
    }
}

fn ai_string(ai: &soapysdr::ArgInfo) -> String {
    format!(
        "key={} value={} name={:?} descr={:?} units={:?} data_type={:?} options={:?}",
        ai.key, ai.value, ai.name, ai.description, ai.units, ai.data_type, ai.options
    )
}

impl Block for SoapySdrSource {
    fn work(&mut self) -> Result<BlockRet> {
        let timeout_us = 10_000;
        let mut o = self.dst.write_buf()?;
        let n = match self.stream.read(&mut [&mut o.slice()], timeout_us) {
            Ok(x) => x,
            Err(e) => {
                if e.code == soapysdr::ErrorCode::Timeout {
                    return Ok(BlockRet::Again);
                }
                return Err(e.into());
            }
        };
        if n > 0 {
            // Tags are always with offset zero.
            o.produce(n, &self.tags);
            self.tags.clear();
        }
        Ok(BlockRet::Again)
    }
}
