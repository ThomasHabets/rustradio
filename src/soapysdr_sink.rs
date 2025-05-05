//! SoapySDR sink.

use crate::Result;
use log::debug;
use soapysdr::Direction;

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
#[derive(Default)]
pub struct SoapySdrSinkBuilder {
    dev: String,
    channel: usize,
    ogain: f64,
    samp_rate: f64,
    freq: f64,
}

impl SoapySdrSinkBuilder {
    /// Build block.
    pub fn build(self, src: ReadStream<Complex>) -> Result<SoapySdrSink> {
        let dev = soapysdr::Device::new(&*self.dev)?;
        debug!("SoapySDR TX driver: {}", dev.driver_key()?);
        debug!("SoapySDR TX hardware: {}", dev.hardware_key()?);
        debug!("SoapySDR TX hardware info: {}", dev.hardware_info()?);
        debug!(
            "SoapySDR TX frontend mapping: {}",
            dev.frontend_mapping(Direction::Tx)?
        );
        debug!("SoapySDR TX clock sources: {:?}", dev.list_clock_sources()?);
        let chans = dev.num_channels(Direction::Tx)?;
        debug!("SoapySDR TX channels : {}", chans);
        for channel in 0..chans {
            debug!(
                "SoapySDR TX channel {channel} antennas: {:?}",
                dev.antennas(Direction::Tx, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} gains: {:?}",
                dev.list_gains(Direction::Tx, channel)?
            );
            debug!(
                "SoapySDR TX channel {channel} frequency range: {:?}",
                dev.frequency_range(Direction::Tx, channel)?
            );
            for ai in dev.stream_args_info(Direction::Tx, channel)? {
                debug!("SoapySDR TX channel {channel} arg info: {}", ai_string(&ai));
            }
            debug!(
                "SoapySDR TX channel {channel} stream formats: {:?}. Native: {:?}",
                dev.stream_formats(Direction::Tx, channel)?,
                dev.native_stream_format(Direction::Tx, channel)?,
            );
            debug!(
                "SoapySDR TX channel {channel} info: {}",
                dev.channel_info(Direction::Tx, channel)?
            );
        }
        dev.set_frequency(
            Direction::Tx,
            self.channel,
            self.freq,
            soapysdr::Args::new(),
        )?;
        dev.set_sample_rate(Direction::Tx, self.channel, self.samp_rate)?;
        dev.set_gain(Direction::Tx, self.channel, self.ogain)?;
        let mut stream = dev.tx_stream(&[self.channel])?;
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
    pub fn builder(dev: String, freq: f64, samp_rate: f64) -> SoapySdrSinkBuilder {
        SoapySdrSinkBuilder {
            dev,
            freq,
            samp_rate,
            ..Default::default()
        }
    }
}

impl Block for SoapySdrSink {
    fn work(&mut self) -> Result<BlockRet> {
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
