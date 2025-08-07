//! SoapySDR sink.

use crate::Result;
use log::debug;

use crate::Complex;
use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;

fn ai_string(ai: &soapysdr::ArgInfo) -> String {
    format!(
        "key={} value={} name={:?} descr={:?} units={:?} data_type={:?} options={:?}",
        ai.key, ai.value, ai.name, ai.description, ai.units, ai.data_type, ai.options
    )
}

/// SoapySDR sink builder.
pub struct SoapySdrSinkBuilder<'a> {
    dev: &'a soapysdr::Device,
    antenna: Option<String>,
    channel: usize,
    ogain: f64,
    samp_rate: f64,
    freq: f64,
}

impl SoapySdrSinkBuilder<'_> {
    /// Set channel number.
    pub fn channel(mut self, channel: usize) -> Self {
        self.channel = channel;
        self
    }
    /// Set input gain.
    ///
    /// Normalized to 0.0 to 1.0.
    pub fn ogain(mut self, igain: f64) -> Self {
        self.ogain = igain;
        self
    }
    /// Set antenna.
    pub fn antenna<T: Into<String>>(mut self, a: T) -> Self {
        self.antenna = Some(a.into());
        self
    }
    /// Build block.
    pub fn build(self, src: ReadStream<Complex>) -> Result<SoapySdrSink> {
        let dir = soapysdr::Direction::Tx;
        debug!("SoapySDR TX driver: {}", self.dev.driver_key()?);
        debug!("SoapySDR TX hardware: {}", self.dev.hardware_key()?);
        debug!("SoapySDR TX hardware info: {}", self.dev.hardware_info()?);
        debug!(
            "SoapySDR TX frontend mapping: {}",
            self.dev.frontend_mapping(dir)?
        );
        for sensor in self.dev.list_sensors()? {
            debug!(
                "SoapySDR TX sensor {sensor}: {:?}",
                self.dev.get_sensor_info(&sensor)?
            );
            debug!(
                "SoapySDR TX sensor {sensor}: {:?}",
                self.dev.read_sensor(&sensor)?
            );
        }
        debug!(
            "SoapySDR TX clock sources: {:?}",
            self.dev.list_clock_sources()?
        );
        debug!(
            "SoapySDR TX active clock source: {:?}",
            self.dev.get_clock_source()?
        );
        let chans = self.dev.num_channels(dir)?;
        debug!("SoapySDR TX channels : {chans}");
        for channel in 0..chans {
            for sensor in self.dev.list_channel_sensors(dir, channel)? {
                match self.dev.read_channel_sensor(dir, channel, &sensor) {
                    Ok(s) => debug!("SoapySDR TX channel sensor {sensor}: {s}"),
                    Err(e) => debug!("SoapySDR TX channel sensor {sensor} error: {e}"),
                }
            }
            debug!(
                "SoapySDR TX channel {channel} antennas: {:?}",
                self.dev.antennas(dir, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} gains: {:?}",
                self.dev.list_gains(dir, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} gain range: {:?}",
                self.dev.gain_range(dir, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} frequency range: {:?}",
                self.dev.frequency_range(dir, channel)?
            );
            for ai in self.dev.stream_args_info(dir, channel)? {
                debug!("SoapySDR TX channel {channel} arg info: {}", ai_string(&ai));
            }
            debug!(
                "SoapySDR TX channel {channel} stream formats: {:?}. Native: {:?}",
                self.dev.stream_formats(dir, channel)?,
                self.dev.native_stream_format(dir, channel)?,
            );
            debug!(
                "SoapySDR TX channel {channel} info: {}",
                self.dev.channel_info(dir, channel)?
            );
        }
        self.dev
            .set_frequency(dir, self.channel, self.freq, soapysdr::Args::new())?;
        self.dev
            .set_sample_rate(dir, self.channel, self.samp_rate)?;
        let gr = self.dev.gain_range(dir, self.channel)?;
        let gain = gr.minimum + self.ogain * (gr.maximum - gr.minimum);
        self.dev.set_gain(dir, self.channel, gain)?;
        if let Some(a) = self.antenna {
            self.dev.set_antenna(dir, self.channel, a)?;
        }
        let mut stream = self.dev.tx_stream(&[self.channel])?;
        stream.activate(None)?;
        Ok(SoapySdrSink { src, stream })
    }
}

#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SoapySdrSink {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    stream: soapysdr::TxStream<Complex>,
}

impl SoapySdrSink {
    /// Create new builder.
    pub fn builder(dev: &soapysdr::Device, freq: f64, samp_rate: f64) -> SoapySdrSinkBuilder<'_> {
        SoapySdrSinkBuilder {
            dev,
            freq,
            samp_rate,
            channel: 0,
            ogain: 0.5,
            antenna: None,
        }
    }
}

impl Block for SoapySdrSink {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let timeout_us = 10_000;
        let (i, _tags) = self.src.read_buf()?;
        let ilen = i.len();
        if ilen == 0 {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        // debug!("writing {}", i.slice().len());
        let n = match self.stream.write(
            &[i.slice()],
            None,  // at_ns
            false, // end_burst
            timeout_us,
        ) {
            Ok(x) => x,
            Err(e) => {
                if e.code == soapysdr::ErrorCode::Timeout {
                    return Ok(BlockRet::Again);
                }
                return Err(e.into());
            }
        };
        i.consume(n);
        if ilen == n {
            Ok(BlockRet::WaitForStream(&self.src, 1))
        } else {
            Ok(BlockRet::Again)
        }
    }
}
