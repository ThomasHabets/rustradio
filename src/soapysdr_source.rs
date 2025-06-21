//! SoapySDR source.
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use log::{debug, trace};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};
use crate::{Complex, Error, Float, Result};

// Sensors and time_ns are re-read this often.
const TIME_TAG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

enum SensorType {
    Float,
    U64,
    Bool,
}

// Allowlist of sensors that don't accidentally reveal secrets.
static ALLOWED_SENSORS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    ["gps_time", "gps_locked", "ref_locked", "lo_locked"]
        .into_iter()
        .collect()
});

// If GPS tags are enabled, these are the sensor names.
//
// Should not be enabled by default, since they can be sensitive.
static POSITION_SENSORS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    ["gps_gpgga", "gps_gprmc", "gps_servo"]
        .into_iter()
        .collect()
});

// If tag is not listed, or fails to parse, then it defaults to String.
static SENSOR_TYPE: LazyLock<HashMap<&str, SensorType>> = LazyLock::new(|| {
    [
        ("temp", SensorType::Float),
        ("rssi", SensorType::Float),
        ("gps_time", SensorType::U64),
        ("ref_locked", SensorType::Bool),
        ("gps_locked", SensorType::Bool),
        ("lo_locked", SensorType::Bool),
    ]
    .into_iter()
    .collect()
});

// Turn a tag value into a typed TagValue. Defaults to String if unknown or
// failing to parse.
fn make_sensor_tag(tag: &str, val: &str) -> TagValue {
    match SENSOR_TYPE.get(tag) {
        Some(SensorType::Float) => val
            .parse::<Float>()
            .map(TagValue::Float)
            .unwrap_or_else(|e| {
                trace!("Failed to parse sensor tag {tag} value {val} as float: {e}");
                TagValue::String(val.to_string())
            }),
        Some(SensorType::U64) => val.parse::<u64>().map(TagValue::U64).unwrap_or_else(|e| {
            trace!("Failed to parse sensor tag {tag} value {val} as u64: {e}");
            TagValue::String(val.to_string())
        }),
        Some(SensorType::Bool) => val.parse::<bool>().map(TagValue::Bool).unwrap_or_else(|e| {
            trace!("Failed to parse sensor tag {tag} value {val} as bool: {e}");
            TagValue::String(val.to_string())
        }),
        None => TagValue::String(val.to_string()),
    }
}

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
    gps_coords: bool,
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
    /// Set whether to generate GPS coordinate tags.
    pub fn gps_coordinates(mut self, v: bool) -> Self {
        self.gps_coords = v;
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
                "SoapySdrSource::input_gain",
                TagValue::Float(self.igain as Float),
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
        // Hardware info has serial numbers.
        debug!("SoapySDR RX hardware info: {}", self.dev.hardware_info()?);
        log_and_tag!(
            tags,
            self.dev.frontend_mapping(soapysdr::Direction::Rx),
            "frontend_mapping"
        );
        log_and_tag!(tags, self.dev.get_clock_source(), "clock_source");
        log_and_tag!(tags, self.dev.get_time_source(), "time_source");
        let allowed_sensors = {
            let mut a = ALLOWED_SENSORS.clone();
            if self.gps_coords {
                a.extend(&*POSITION_SENSORS);
            }
            a
        };
        for sensor in self.dev.list_sensors()? {
            debug!(
                "SoapySDR RX sensor {sensor}: {:?}",
                self.dev.get_sensor_info(&sensor)?
            );
            let read = self.dev.read_sensor(&sensor)?.to_string();
            debug!("SoapySDR RX sensor {sensor}: {read:?}");
        }
        debug!(
            "SoapySDR RX clock sources: {:?}",
            self.dev.list_clock_sources()?
        );
        debug!(
            "SoapySDR RX time sources: {:?}",
            self.dev.list_time_sources()?
        );
        if let Ok(t) = self.dev.get_hardware_time(None) {
            tags.push(Tag::new(
                0,
                "SoapySdrSource::hardware_time",
                TagValue::I64(t),
            ));
        }
        let chans = self.dev.num_channels(soapysdr::Direction::Rx)?;
        debug!("SoapySDR RX channels : {chans}");
        for channel in 0..chans {
            for sensor in self
                .dev
                .list_channel_sensors(soapysdr::Direction::Rx, channel)?
            {
                match self
                    .dev
                    .read_channel_sensor(soapysdr::Direction::Rx, channel, &sensor)
                {
                    Ok(s) => debug!("SoapySDR RX channel {channel} sensor {sensor}: {s}"),
                    Err(e) => debug!("SoapySDR RX channel {channel} sensor {sensor} error: {e}"),
                }
            }
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
        Ok((
            SoapySdrSource {
                dev: self.dev.clone(),
                channel: self.channel,
                allowed_sensors,
                stream,
                dst,
                tags,
                last_time_tag: None,
            },
            dr,
        ))
    }
}

/// SoapySDR source.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SoapySdrSource {
    dev: soapysdr::Device,
    channel: usize,
    allowed_sensors: HashSet<&'static str>,
    stream: soapysdr::RxStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
    #[rustradio(default)]
    tags: Vec<Tag>,

    #[rustradio(default)]
    last_time_tag: Option<std::time::Instant>,
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
            gps_coords: false,
        }
    }
    fn add_sensor_tags(&mut self) -> Result<()> {
        self.dev
            .list_sensors()?
            .into_iter()
            .filter(|sensor| {
                let s: &str = sensor;
                self.allowed_sensors.contains(s)
            })
            .map(|sensor| {
                self.dev.read_sensor(&sensor).map(|s| {
                    self.tags.push(Tag::new(
                        0,
                        format!("SoapySdrSource::sensor_{sensor}"),
                        make_sensor_tag(&sensor, &s),
                    ));
                })
            })
            .for_each(|r| {
                if let Err(e) = r {
                    debug!("SoapySdrSource failed to attach sensor tags: {e}");
                }
            });
        Ok(())
    }
    fn add_channel_sensor_tags(&mut self) -> Result<()> {
        self.dev
            .list_channel_sensors(soapysdr::Direction::Rx, self.channel)?
            .into_iter()
            .filter(|sensor| {
                let s: &str = sensor;
                self.allowed_sensors.contains(s)
            })
            .map(|sensor| {
                (
                    sensor.clone(),
                    self.dev
                        .read_channel_sensor(soapysdr::Direction::Rx, self.channel, &sensor)
                        .map(|s| {
                            self.tags.push(Tag::new(
                                0,
                                format!("SoapySdrSource::sensor_channel_{sensor}"),
                                make_sensor_tag(&sensor, &s),
                            ));
                        }),
                )
            })
            .for_each(|r| {
                if let (s, Err(e)) = r {
                    debug!("SoapySdrSource failed to attach channel sensor tag {s}: {e}");
                }
            });
        Ok(())
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
            if match self.last_time_tag {
                None => true,
                Some(x) if x.elapsed() > TIME_TAG_INTERVAL => true,
                _ => false,
            } {
                let time_ns = self.stream.time_ns();
                self.tags.push(Tag::new(
                    0,
                    "SoapySdrSource::time_ns",
                    TagValue::I64(time_ns),
                ));
                if let Err(e) = self.add_sensor_tags() {
                    debug!("SoapySdrSource failed to attach sensor tags: {e}");
                }
                if let Err(e) = self.add_channel_sensor_tags() {
                    debug!("SoapySdrSource failed to attach channel sensor tags: {e}");
                }
                self.last_time_tag = Some(std::time::Instant::now());
            }
            // Tags are always with offset zero.
            o.produce(n, &self.tags);
            self.tags.clear();
        }
        Ok(BlockRet::Again)
    }
}
