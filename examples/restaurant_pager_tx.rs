//! Transmit a finite burst of restaurant guest-pager messages with SoapySDR.
//!
//! The generated packets match the 25-bit EV1527 variant decoded by
//! `examples/restaurant_pager.rs`. Each `--message` is `PAGER:FUNCTION`, where
//! the function is `buzz`, `sync`, or a numeric value from 0 through 15.
//!
//! ```text
//! cargo run --release --features soapysdr --example restaurant_pager_tx -- \
//!     --driver 'driver=lime' \
//!     --system-id 0xf9bf \
//!     --message 11:buzz \
//!     --message 3:sync
//! ```
//!
//! Ensure the selected frequency and transmission are legal in your location.

use std::str::FromStr;

use anyhow::{Result, ensure};
use clap::Parser;

use rustradio::blocks::{Map, PwmEncoder, PwmGapPulse, SoapySdrSink};
use rustradio::graph::{Graph, GraphRunner};
use rustradio::stream::{Tag, TagValue, new_nocopy_stream};
use rustradio::{Complex, Float, parse_frequency, parse_verbosity};

const SHORT_US: u32 = 204;
const LONG_US: u32 = 636;
const ROW_GAP_US: u32 = 880;
const RESET_US: u32 = 7_312;
const FRAME_BITS: usize = 25;

/// One pager number and function requested on the command line.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PagerMessage {
    pager: u8,
    function: u8,
}

impl PagerMessage {
    /// Return a readable function name for logging.
    fn function_name(&self) -> &'static str {
        match self.function {
            0x0d => "Buzz",
            0x0f => "Sync",
            _ => "Custom",
        }
    }
}

impl FromStr for PagerMessage {
    type Err = String;

    /// Parse `PAGER:FUNCTION`, accepting named or numeric functions.
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (pager, function) = value
            .split_once(':')
            .ok_or_else(|| "message must be PAGER:FUNCTION, such as 11:buzz".to_string())?;
        let pager = parse_integer(pager)?;
        if pager > 0x0f {
            return Err("pager number must be between 0 and 15".to_string());
        }
        let function = match function.to_ascii_lowercase().as_str() {
            "buzz" => 0x0d,
            "sync" => 0x0f,
            _ => parse_integer(function)?,
        };
        if function > 0x0f {
            return Err("pager function must be between 0 and 15".to_string());
        }
        Ok(Self {
            pager: pager as u8,
            function: function as u8,
        })
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal integer.
fn parse_integer(value: &str) -> std::result::Result<u32, String> {
    let hexadecimal = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    match hexadecimal {
        Some(value) => u32::from_str_radix(value, 16),
        None => value.parse(),
    }
    .map_err(|error| format!("invalid integer {value:?}: {error}"))
}

/// Parse and range-check the 16-bit pager-system identifier.
fn parse_system_id(value: &str) -> std::result::Result<u16, String> {
    let value = parse_integer(value)?;
    u16::try_from(value).map_err(|_| "system ID must fit in 16 bits".to_string())
}

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Transmit restaurant guest-pager message bursts with SoapySDR"
)]
struct Opt {
    /// SoapySDR device string.
    #[arg(short, long, default_value = "driver=lime")]
    driver: String,

    /// RF center frequency in Hz.
    #[arg(long, value_parser = parse_frequency, default_value = "433.92M")]
    frequency: f64,

    /// SDR sample rate in samples per second.
    #[arg(long, value_parser = parse_frequency, default_value = "125k")]
    sample_rate: f64,

    /// Normalized transmit gain from zero through one.
    #[arg(long, default_value_t = 0.1)]
    gain: f64,

    /// Complex baseband amplitude while the OOK carrier is on.
    #[arg(long, default_value_t = 0.5)]
    amplitude: Float,

    /// Pager system identifier, in decimal or hexadecimal.
    #[arg(long, value_parser = parse_system_id, default_value = "0xf9bf")]
    system_id: u16,

    /// Pager message in PAGER:FUNCTION form; may be specified more than once.
    #[arg(
        short,
        long,
        value_name = "PAGER:FUNCTION",
        required_unless_present = "list_devices"
    )]
    message: Vec<PagerMessage>,

    /// Number of identical frames sent for each message.
    #[arg(long, default_value_t = 8)]
    repeats: usize,

    /// SoapySDR transmit channel.
    #[arg(long, default_value_t = 0)]
    channel: usize,

    /// SoapySDR transmit antenna name.
    #[arg(long)]
    antenna: Option<String>,

    /// List SoapySDR devices and exit.
    #[arg(long)]
    list_devices: bool,

    /// Logging verbosity.
    #[arg(short, value_parser = parse_verbosity, default_value = "info")]
    verbose: usize,
}

/// Convert a protocol duration to the nearest positive sample count.
fn us_to_samples(sample_rate: f64, micros: u32) -> Result<usize> {
    let samples = (sample_rate * f64::from(micros) / 1_000_000.0).round();
    ensure!(
        samples >= 1.0 && samples.is_finite() && samples <= usize::MAX as f64,
        "{micros} us does not fit the selected sample rate",
    );
    Ok(samples as usize)
}

/// Pack the restaurant-pager fields and return 25 MSB-first bits.
fn encode_message(system_id: u16, message: &PagerMessage) -> (u32, Vec<u8>) {
    let raw = (u32::from(system_id) << 9)
        | (u32::from(message.pager) << 5)
        | (u32::from(message.function) << 1)
        | 1;
    let bits = (0..FRAME_BITS)
        .rev()
        .map(|shift| ((raw >> shift) & 1) as u8)
        .collect();
    (raw, bits)
}

fn main() -> Result<()> {
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .module("soapysdr")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    soapysdr::configure_logging();

    if opt.list_devices {
        for device in soapysdr::enumerate("")? {
            println!("{device}");
        }
        return Ok(());
    }

    ensure!(
        opt.frequency > 0.0 && opt.frequency.is_finite(),
        "frequency must be finite and greater than zero",
    );
    ensure!(
        opt.sample_rate > 0.0 && opt.sample_rate.is_finite(),
        "sample rate must be finite and greater than zero",
    );
    ensure!(
        (0.0..=1.0).contains(&opt.gain) && opt.gain.is_finite(),
        "gain must be finite and between zero and one",
    );
    ensure!(
        opt.amplitude > 0.0 && opt.amplitude <= 1.0 && opt.amplitude.is_finite(),
        "amplitude must be finite, greater than zero, and no greater than one",
    );
    ensure!(opt.repeats > 0, "repeat count must be greater than zero");

    let short = us_to_samples(opt.sample_rate, SHORT_US)?;
    let long = us_to_samples(opt.sample_rate, LONG_US)?;
    let frame_gap = us_to_samples(opt.sample_rate, ROW_GAP_US)?;
    let reset_gap = us_to_samples(opt.sample_rate, RESET_US)?;

    let (packets, packet_stream) = new_nocopy_stream();
    for message in &opt.message {
        let (raw, bits) = encode_message(opt.system_id, message);
        println!(
            "Queueing id=0x{:04x} pager={} function={} (0x{:x}) raw=0x{raw:07x}",
            opt.system_id,
            message.pager,
            message.function_name(),
            message.function,
        );
        packets.push(
            bits,
            vec![Tag::new(
                0,
                "RestaurantPagerTx::message",
                TagValue::String(format!(
                    "id=0x{:04x} pager={} function=0x{:x}",
                    opt.system_id, message.pager, message.function,
                )),
            )],
        );
    }
    drop(packets);

    let device = soapysdr::Device::new(&*opt.driver)?;
    let mut graph = Graph::new();
    let (encoder, envelope) = PwmEncoder::builder(short, long, frame_gap, reset_gap)
        .repeats(opt.repeats)
        .max_frame_bits(FRAME_BITS)
        .gap_pulse(PwmGapPulse::Delimiter)
        .build(packet_stream)?;
    graph.add(Box::new(encoder));
    let amplitude = opt.amplitude;
    let (to_complex, samples) = Map::keep_tags(envelope, "OokToComplex", move |level| {
        Complex::new(amplitude * level, 0.0)
    });
    graph.add(Box::new(to_complex));
    let mut sink = SoapySdrSink::builder(&device, opt.frequency, opt.sample_rate)
        .channel(opt.channel)
        .ogain(opt.gain);
    if let Some(antenna) = opt.antenna {
        sink = sink.antenna(antenna);
    }
    graph.add(Box::new(sink.build(samples)?));

    println!(
        "Transmitting {} message(s), {} frame(s) each at {} Hz",
        opt.message.len(),
        opt.repeats,
        opt.frequency,
    );
    graph.run()?;
    println!("Transmission complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the transmitter and receiver agree on the field layout.
    #[test]
    fn message_layout_matches_decoder() {
        let message = PagerMessage {
            pager: 11,
            function: 0x0d,
        };
        let (raw, bits) = encode_message(0xf9bf, &message);
        assert_eq!(bits.len(), FRAME_BITS);
        assert_eq!(bits.last(), Some(&1));
        assert_eq!((raw >> 9) & 0xffff, 0xf9bf);
        assert_eq!((raw >> 5) & 0x0f, 11);
        assert_eq!((raw >> 1) & 0x0f, 0x0d);
    }

    /// Verify named, decimal, and hexadecimal message forms.
    #[test]
    fn parses_messages() {
        assert_eq!(
            "11:buzz".parse(),
            Ok(PagerMessage {
                pager: 11,
                function: 0x0d,
            })
        );
        assert_eq!(
            "0xf:0x2".parse(),
            Ok(PagerMessage {
                pager: 15,
                function: 2,
            })
        );
        assert!("16:sync".parse::<PagerMessage>().is_err());
        assert!("1:16".parse::<PagerMessage>().is_err());
        assert!("buzz".parse::<PagerMessage>().is_err());
    }
}
