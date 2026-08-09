//! Decode restaurant guest pagers from a complex-float I/Q capture.
//!
//! The supported protocol is the 25-bit EV1527 variant described by rtl_433's
//! [`restaurant_pager.conf`][protocol]: short OOK pulses are one bits, long
//! pulses are zero bits, and every frame ends with a delimiter pulse followed
//! by a long gap.
//!
//! ```text
//! cargo run --release --example restaurant_pager -- \
//!     data.c32
//! ```
//!
//! [protocol]: https://github.com/jflaflamme/rtl_433/blob/1b5550e75a2c1f483db1fb29e80173356bbb74be/conf/restaurant_pager.conf

use std::path::PathBuf;

use anyhow::{Result, ensure};
use clap::Parser;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::{ComplexToMag2, FileSource, PwmDecoder, PwmFrame, PwmGapPulse};
use rustradio::graph::{Graph, GraphRunner};
use rustradio::stream::NCReadStream;
use rustradio::{Complex, Float};

const SHORT_US: u32 = 204;
const LONG_US: u32 = 636;
const ROW_GAP_US: u32 = 880;
const RESET_US: u32 = 7_312;
const FRAME_BITS: usize = 25;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Raw little-endian complex-f32 I/Q capture.
    input: PathBuf,

    /// Capture sample rate in samples per second.
    #[arg(long, default_value_t = 125_000)]
    sample_rate: u32,

    /// OOK power threshold (the input is magnitude squared).
    #[arg(long, default_value_t = 0.01)]
    threshold: Float,

    /// Number of identical frames required in one transmission.
    #[arg(long, default_value_t = 3)]
    repeats: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DecodedTransmission {
    raw: u32,
    repeats: usize,
    first_sample: u64,
    sample_rate: u32,
}

impl DecodedTransmission {
    /// Interpret a generic PWM frame as a restaurant-pager message.
    fn from_frame(frame: PwmFrame, sample_rate: u32) -> Option<Self> {
        if frame.len() != FRAME_BITS || frame.bits().last() != Some(&1) {
            return None;
        }
        let raw = frame
            .bits()
            .iter()
            .fold(0_u32, |value, &bit| (value << 1) | u32::from(bit));
        Some(Self {
            raw,
            repeats: frame.repeats(),
            first_sample: frame.first_sample(),
            sample_rate,
        })
    }

    /// Return the pager system identifier.
    fn system_id(&self) -> u16 {
        ((self.raw >> 9) & 0xffff) as u16
    }

    /// Return the addressed pager number.
    fn pager(&self) -> u8 {
        ((self.raw >> 5) & 0x0f) as u8
    }

    /// Return the requested pager function code.
    fn function(&self) -> u8 {
        ((self.raw >> 1) & 0x0f) as u8
    }

    /// Return a readable name for the pager function.
    fn function_name(&self) -> &'static str {
        match self.function() {
            0x0d => "Buzz",
            0x0f => "Sync",
            _ => "Unknown",
        }
    }
}

impl std::fmt::Display for DecodedTransmission {
    /// Format the decoded pager fields and capture timestamp.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.first_sample as f64 / f64::from(self.sample_rate);
        write!(
            f,
            "Restaurant-Pager: id=0x{:04x} pager={} function={} (0x{:x}) \
             repeats={} raw=0x{:07x} time={seconds:.6}s",
            self.system_id(),
            self.pager(),
            self.function_name(),
            self.function(),
            self.repeats,
            self.raw,
        )
    }
}

/// Convert a duration in microseconds to a rounded input-sample count.
fn us_to_samples(sample_rate: u32, micros: u32) -> usize {
    ((u64::from(sample_rate) * u64::from(micros) + 500_000) / 1_000_000)
        .max(1)
        .try_into()
        .expect("sample count does not fit in usize")
}

/// Print decoded messages without coupling console I/O to the DSP block.
#[derive(rustradio_macros::Block)]
#[rustradio(new)]
struct RestaurantPagerPrinter {
    #[rustradio(in)]
    src: NCReadStream<PwmFrame>,
    sample_rate: u32,
}

impl Block for RestaurantPagerPrinter {
    /// Print all decoded frames currently available on the input stream.
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        loop {
            let Some((frame, _tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            if let Some(decoded) = DecodedTransmission::from_frame(frame, self.sample_rate) {
                println!("{decoded}");
            }
        }
    }
}

/// Build and run the restaurant-pager decoding graph.
fn main() -> Result<()> {
    let opt = Opt::parse();
    ensure!(opt.sample_rate > 0, "sample rate must be greater than zero");
    let mut graph = Graph::new();

    let prev = rustradio::blockchain![
        graph,
        prev,
        FileSource::<Complex>::new(&opt.input)?,
        ComplexToMag2::new(prev),
        PwmDecoder::builder(
            opt.threshold,
            us_to_samples(opt.sample_rate, SHORT_US),
            us_to_samples(opt.sample_rate, LONG_US),
            us_to_samples(opt.sample_rate, ROW_GAP_US),
            us_to_samples(opt.sample_rate, RESET_US),
        )
        .frame_bits(Some(FRAME_BITS))
        .min_repeats(opt.repeats)
        .gap_pulse(PwmGapPulse::Delimiter)
        .build(prev)?,
    ];
    graph.add(Box::new(RestaurantPagerPrinter::new(prev, opt.sample_rate)));

    graph.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the decoded bit fields match the restaurant-pager layout.
    #[test]
    fn decoded_fields() {
        let target = (0xf9bf_u32 << 9) | (11 << 5) | (0x0d << 1) | 1;
        let decoded = DecodedTransmission {
            raw: target,
            repeats: 3,
            first_sample: 0,
            sample_rate: 125_000,
        };
        assert_eq!(decoded.system_id(), 0xf9bf);
        assert_eq!(decoded.pager(), 11);
        assert_eq!(decoded.function(), 0x0d);
        assert_eq!(decoded.function_name(), "Buzz");
    }
}
