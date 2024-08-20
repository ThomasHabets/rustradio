//! SoapySDR source.
use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::{Complex, Error};

impl From<soapysdr::Error> for Error {
    fn from(e: soapysdr::Error) -> Self {
        Error::new(&format!("Soapy SDR Error: {}", e))
    }
}

/// SoapySDR source builder.
#[derive(Default)]
pub struct SoapySdrSourceBuilder {
    dev: String,
    channel: usize,
    igain: f64,
    samp_rate: f64,
    freq: f64,
}

impl SoapySdrSourceBuilder {
    /// Create new builder.
    pub fn new(dev: String, freq: f64, samp_rate: f64) -> Self {
        Self {
            dev,
            freq,
            samp_rate,
            ..Default::default()
        }
    }
    /// Set channel number.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }
    /// Set input gain.
    pub fn igain(mut self, igain: f64) -> Self {
        self.igain = igain;
        self
    }
    /// Build the source object.
    pub fn build(self) -> Result<SoapySdrSource> {
        let dev = soapysdr::Device::new(&*self.dev)?;
        debug!("SoapySDR driver: {}", dev.driver_key()?);
        debug!("SoapySDR hardware: {}", dev.hardware_key()?);
        debug!("SoapySDR hardware info: {}", dev.hardware_info()?);
        debug!(
            "SoapySDR frontend mapping: {}",
            dev.frontend_mapping(soapysdr::Direction::Rx)?
        );
        let chans = dev.num_channels(soapysdr::Direction::Rx)?;
        debug!("SoapySDR RX channels : {}", chans);
        for channel in 0..chans {
            debug!(
                "SoapySDR channel {channel} antennas: {:?}",
                dev.antennas(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR channel {channel} gains: {:?}",
                dev.list_gains(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR channel {channel} frequency range: {:?}",
                dev.frequency_range(soapysdr::Direction::Rx, channel)?
            );
            for ai in dev.stream_args_info(soapysdr::Direction::Rx, channel)? {
                debug!("SoapySDR channel {channel} arg info: {}", ai_string(&ai));
            }
            debug!(
                "SoapySDR channel {channel} stream formats: {:?}",
                dev.stream_formats(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR channel {channel} info: {}",
                dev.channel_info(soapysdr::Direction::Rx, channel)?
            );
        }
        dev.set_frequency(
            soapysdr::Direction::Rx,
            self.channel,
            self.freq,
            soapysdr::Args::new(),
        )?;
        dev.set_sample_rate(soapysdr::Direction::Rx, self.channel, self.samp_rate)?;
        dev.set_gain(soapysdr::Direction::Rx, self.channel, self.igain)?;
        let mut stream = dev.rx_stream(&[self.channel])?;
        stream.activate(None)?;
        Ok(SoapySdrSource {
            stream,
            dst: Stream::newp(),
        })
    }
}

/// SoapySDR source.
pub struct SoapySdrSource {
    stream: soapysdr::RxStream<Complex>,
    dst: Streamp<Complex>,
}

fn ai_string(ai: &soapysdr::ArgInfo) -> String {
    format!(
        "key={} value={} name={:?} descr={:?} units={:?} data_type={:?} options={:?}",
        ai.key, ai.value, ai.name, ai.description, ai.units, ai.data_type, ai.options
    )
}

impl SoapySdrSource {
    /// Get output stream.
    pub fn out(&self) -> Streamp<Complex> {
        self.dst.clone()
    }
}

impl Block for SoapySdrSource {
    fn block_name(&self) -> &str {
        "SoapySdrSource"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let timeout_us = 10_000;
        let mut o = self.dst.write_buf()?;
        let n = match self.stream.read(&mut [&mut o.slice()], timeout_us) {
            Ok(x) => x,
            Err(e) => {
                if e.code == soapysdr::ErrorCode::Timeout {
                    return Ok(BlockRet::Ok);
                }
                return Err(e.into());
            }
        };
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}
