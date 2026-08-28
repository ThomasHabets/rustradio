//! SoapySDR source.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, mpsc};

use log::{debug, trace, warn};

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

/// Read one batch of device and channel sensors.
fn read_sensor_tags(
    dev: &soapysdr::Device,
    channel: usize,
    allowed_sensors: &HashSet<&str>,
) -> Vec<Tag> {
    let mut tags = Vec::new();
    match dev.list_sensors() {
        Ok(sensors) => {
            for sensor in sensors {
                if !allowed_sensors.contains(sensor.as_str()) {
                    continue;
                }
                match dev.read_sensor(&sensor) {
                    Ok(value) => tags.push(Tag::new(
                        0,
                        format!("SoapySdrSource::sensor_{sensor}"),
                        make_sensor_tag(&sensor, &value),
                    )),
                    Err(error) => {
                        debug!("SoapySdrSource failed to read sensor {sensor}: {error}");
                    }
                }
            }
        }
        Err(error) => debug!("SoapySdrSource failed to list sensors: {error}"),
    }

    match dev.list_channel_sensors(soapysdr::Direction::Rx, channel) {
        Ok(sensors) => {
            for sensor in sensors {
                if !allowed_sensors.contains(sensor.as_str()) {
                    continue;
                }
                match dev.read_channel_sensor(soapysdr::Direction::Rx, channel, &sensor) {
                    Ok(value) => tags.push(Tag::new(
                        0,
                        format!("SoapySdrSource::sensor_channel_{sensor}"),
                        make_sensor_tag(&sensor, &value),
                    )),
                    Err(error) => debug!(
                        "SoapySdrSource failed to read channel {channel} sensor {sensor}: {error}"
                    ),
                }
            }
        }
        Err(error) => {
            debug!("SoapySdrSource failed to list channel {channel} sensors: {error}");
        }
    }
    tags
}

/// Poll SoapySDR sensors without blocking the sample-read path.
struct SensorPoller {
    latest_tags: Arc<Mutex<Vec<Tag>>>,
    shutdown: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SensorPoller {
    fn new(
        dev: soapysdr::Device,
        channel: usize,
        allowed_sensors: HashSet<&'static str>,
    ) -> Result<Self> {
        Ok(Self::spawn_with(TIME_TAG_INTERVAL, move || {
            read_sensor_tags(&dev, channel, &allowed_sensors)
        })?)
    }

    fn spawn_with<F>(interval: std::time::Duration, mut poll: F) -> std::io::Result<Self>
    where
        F: FnMut() -> Vec<Tag> + Send + 'static,
    {
        let latest_tags = Arc::new(Mutex::new(Vec::new()));
        let thread_tags = Arc::clone(&latest_tags);
        let (shutdown, shutdown_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("SoapySdrSource-sensors".to_string())
            .spawn(move || {
                loop {
                    let tags = poll();
                    if !tags.is_empty() {
                        let Ok(mut latest) = thread_tags.lock() else {
                            warn!("SoapySdrSource sensor-tag lock was poisoned");
                            return;
                        };
                        *latest = tags;
                    }
                    match shutdown_rx.recv_timeout(interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            })?;
        Ok(Self {
            latest_tags,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    fn take_tags(&self) -> Vec<Tag> {
        match self.latest_tags.lock() {
            Ok(mut latest) => std::mem::take(&mut *latest),
            Err(_) => {
                warn!("SoapySdrSource sensor-tag lock was poisoned");
                Vec::new()
            }
        }
    }

    fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take()
            && let Err(error) = thread.join()
        {
            warn!("SoapySdrSource sensor thread panicked: {error:?}");
        }
    }
}

impl Drop for SensorPoller {
    fn drop(&mut self) {
        self.stop();
    }
}

impl From<soapysdr::Error> for Error {
    fn from(e: soapysdr::Error) -> Self {
        Error::device(e, "soapysdr")
    }
}

/// SoapySDR source builder.
#[must_use]
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
    /// Set channel number. Default is 0.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }
    /// Set input gain.
    ///
    /// Normalized to 0.0 to 1.0.
    pub fn igain(mut self, igain: f64) -> Result<Self> {
        if igain < 0.0 || igain > 1.0 {
            return Err(Error::msg("input gain must be in range 0.0 - 1.0"));
        }
        self.igain = igain;
        Ok(self)
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
                debug!("SoapySDR RX channel {channel} sensor: {sensor}");
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
        let mut args = soapysdr::Args::new();
        if false {
            args.set("OFFSET", "1e6");
        }
        self.dev
            .set_frequency(soapysdr::Direction::Rx, self.channel, self.freq, args)?;
        self.dev
            .set_sample_rate(soapysdr::Direction::Rx, self.channel, self.samp_rate)?;
        let gr = self.dev.gain_range(soapysdr::Direction::Rx, self.channel)?;
        let gain = gr.minimum + self.igain * (gr.maximum - gr.minimum);
        let gain = gain.min(gr.maximum).max(gr.minimum);
        debug!(
            "SoapySdrSource: input gain {} in range {}-{} became {gain}",
            self.igain, gr.minimum, gr.maximum
        );
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
        let sensor_poller = SensorPoller::new(self.dev.clone(), self.channel, allowed_sensors)?;
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            SoapySdrSource {
                _dev: self.dev.clone(),
                sensor_poller,
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
///
/// Allowed device and receive-channel sensors are polled once per second on a
/// dedicated thread, so slow sensor calls do not block sample reads. The most
/// recent completed sensor batch is attached to the next produced samples at
/// position zero. The polling thread is stopped and joined when the source is
/// dropped.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SoapySdrSource {
    _dev: soapysdr::Device,
    sensor_poller: SensorPoller,
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
    pub fn builder(dev: &soapysdr::Device, freq: f64, samp_rate: f64) -> SoapySdrSourceBuilder<'_> {
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
}

fn ai_string(ai: &soapysdr::ArgInfo) -> String {
    format!(
        "key={} value={} name={:?} descr={:?} units={:?} data_type={:?} options={:?}",
        ai.key, ai.value, ai.name, ai.description, ai.units, ai.data_type, ai.options
    )
}

impl Block for SoapySdrSource {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let timeout_us = 10_000;
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let n = match self.stream.read(&mut [&mut o.slice()], timeout_us) {
            Ok(x) => x,
            Err(e) => {
                if e.code == soapysdr::ErrorCode::Timeout {
                    return Ok(BlockRet::Pending);
                }
                if e.code == soapysdr::ErrorCode::Overflow {
                    warn!("SoapySdrSource: overflow");
                    return Ok(BlockRet::Pending);
                }
                return Err(e.into());
            }
        };
        if n == 0 {
            return Ok(BlockRet::Pending);
        }
        if n > 0 {
            self.tags.extend(self.sensor_poller.take_tags());
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
                self.last_time_tag = Some(std::time::Instant::now());
            }
            // Tags are always with offset zero.
            o.produce(n, &self.tags);
            self.tags.clear();
        }
        Ok(BlockRet::Again)
    }
}

impl Drop for SoapySdrSource {
    fn drop(&mut self) {
        self.sensor_poller.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct ExitFlag(Arc<AtomicBool>);

    impl Drop for ExitFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// Verify sensor tags cross the thread boundary and dropping wakes and
    /// joins a poller waiting for its next interval.
    #[test]
    fn sensor_poller_hands_off_tags_and_stops() {
        let exited = Arc::new(AtomicBool::new(false));
        let exit_flag = ExitFlag(Arc::clone(&exited));
        let (polled, first_poll) = mpsc::sync_channel(1);
        let poller = SensorPoller::spawn_with(std::time::Duration::from_secs(30), move || {
            let _exit_flag = &exit_flag;
            polled.send(()).expect("test receiver should remain open");
            vec![Tag::new(
                0,
                "SoapySdrSource::sensor_ref_locked",
                TagValue::Bool(true),
            )]
        })
        .expect("sensor poller should start");

        first_poll
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("sensor poller did not run");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let tags = loop {
            let tags = poller.take_tags();
            if !tags.is_empty() {
                break tags;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sensor tags were not handed to the source"
            );
            std::thread::yield_now();
        };
        assert_eq!(
            tags,
            vec![Tag::new(
                0,
                "SoapySdrSource::sensor_ref_locked",
                TagValue::Bool(true),
            )]
        );
        assert!(poller.take_tags().is_empty());

        drop(poller);
        assert!(exited.load(Ordering::SeqCst));
    }
}
