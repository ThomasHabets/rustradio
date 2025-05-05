//! SoapySDR source.
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Result};

impl From<soapysdr::Error> for Error {
    fn from(e: soapysdr::Error) -> Self {
        Error::device(e, "soapysdr")
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
    pub fn build(self) -> Result<(SoapySdrSource, ReadStream<Complex>)> {
        let dev = soapysdr::Device::new(&*self.dev)?;
        debug!("SoapySDR RX driver: {}", dev.driver_key()?);
        debug!("SoapySDR RX hardware: {}", dev.hardware_key()?);
        debug!("SoapySDR RX hardware info: {}", dev.hardware_info()?);
        debug!(
            "SoapySDR RX frontend mapping: {}",
            dev.frontend_mapping(soapysdr::Direction::Rx)?
        );
        let chans = dev.num_channels(soapysdr::Direction::Rx)?;
        debug!("SoapySDR RX channels : {}", chans);
        for channel in 0..chans {
            debug!(
                "SoapySDR RX channel {channel} antennas: {:?}",
                dev.antennas(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} gains: {:?}",
                dev.list_gains(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} frequency range: {:?}",
                dev.frequency_range(soapysdr::Direction::Rx, channel)?
            );
            for ai in dev.stream_args_info(soapysdr::Direction::Rx, channel)? {
                debug!("SoapySDR RX channel {channel} arg info: {}", ai_string(&ai));
            }
            debug!(
                "SoapySDR RX channel {channel} stream formats: {:?}",
                dev.stream_formats(soapysdr::Direction::Rx, channel)?
            );
            debug!(
                "SoapySDR RX channel {channel} info: {}",
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
        let (dst, dr) = crate::stream::new_stream();
        Ok((SoapySdrSource { stream, dst }, dr))
    }
}

/// SoapySDR source.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SoapySdrSource {
    stream: soapysdr::RxStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl SoapySdrSource {
    /// Create new SoapySdrSource builder.
    pub fn builder(dev: String, freq: f64, samp_rate: f64) -> SoapySdrSourceBuilder {
        SoapySdrSourceBuilder {
            dev,
            freq,
            samp_rate,
            ..Default::default()
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
        o.produce(n, &[]);
        Ok(BlockRet::Again)
    }
}
