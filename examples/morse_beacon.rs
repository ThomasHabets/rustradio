//! Morse code beacon.
//!
//! ```text
//! cargo run -F soapysdr --example morse_beacon -- \
//!     --sample-rate 320k \
//!     --freq 436.6m \
//!     -d 'driver=uhd' \
//!     --ogain 0.8 \
//!     --clock-source gpsdo \
//!     'M0XXX TESTING'
//! ```
use anyhow::Result;
use clap::Parser;

use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::{Complex, Float};
use rustradio::{parse_frequency, parse_verbosity};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// SoapySDR driver string.
    #[arg(short)]
    driver: String,

    /// Verbosity level.
    #[arg(short, value_parser=parse_verbosity, default_value = "info")]
    verbose: usize,

    /// TX/RX frequency.
    #[arg(long, value_parser=parse_frequency)]
    freq: f64,

    /// SDR sample rate.
    #[arg(long, value_parser=parse_frequency, default_value = "300k")]
    sample_rate: f64,

    /// Output gain. 0.0-1.0.
    #[arg(long, default_value_t = 0.0)]
    ogain: f64,

    /// Amplitude.
    #[arg(long, default_value_t = 1.0)]
    amplitude: f64,

    /// Morse code speed in words per minute.
    #[arg(long, default_value_t = 20.0)]
    wpm: f32,

    /// Set clock source. Valid values are SDR dependent.
    #[arg(long)]
    clock_source: Option<String>,

    /// Message to beacon out.
    msg: String,
}

pub fn main() -> Result<()> {
    println!("Morse code beacon");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .module("soapysdr")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    if opt.msg.is_empty() {
        return Err(anyhow::Error::msg("Beacon message must not be empty"));
    }
    soapysdr::configure_logging();
    let dev = soapysdr::Device::new(&*opt.driver)?;
    if let Some(clock) = &opt.clock_source {
        dev.set_clock_source(clock.as_bytes())?;
    }
    let mut g = MTGraph::new();
    if false {
        // Receiver. Disabled for now.
        use rustradio::file_sink::Mode;
        let prev = blockchain![
            g,
            prev,
            SoapySdrSource::builder(&dev, 739_500_000.0, 300_000.0)
                .igain(0.7)
                .build()?,
        ];
        let mode = Mode::Overwrite;
        g.add(Box::new(
            FileSink::builder("morse-300ksps.c32")
                .mode(mode)
                .build(prev)?,
        ));
    }

    // TODO: enable once
    // <https://github.com/kevinmehall/rust-soapysdr/pull/41> is merged.
    /*
    let dev2 = dev.clone();
    std::thread::spawn(move || {
        loop {
            log::debug!("GPS status: {}: time: {}",
                       dev2.read_sensor("gps_locked").unwrap_or("<error>".to_string()),
                       dev2.read_sensor("gps_time").unwrap_or("<error>".to_string()),
                       );
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
    */

    let amp = opt.amplitude;
    // 20 WPM is 60ms time unit.
    let raw_sps = (opt.wpm / 20.0) / 0.06;
    let prev = blockchain![
        g,
        prev,
        Strobe::new(std::time::Duration::from_secs(10), &opt.msg),
        MorseEncode::new(prev),
        PduToStream::new(prev),
        RationalResampler::builder()
            // Multiply by 100 to get more significant digits on raw SPS, which
            // otherwise rounds 20 WPM 16.666 to 16.
            .deci((100.0 * raw_sps) as usize)
            .interp((100.0 * opt.sample_rate) as usize)
            .build(prev)?,
        Map::keep_tags(prev, "ToComplex", move |s| Complex::new(
            amp as Float * s as Float,
            0.0
        )),
    ];
    g.add(Box::new(
        SoapySdrSink::builder(&dev, opt.freq, opt.sample_rate)
            .ogain(opt.ogain)
            .build(prev)?,
    ));
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");
    g.run()?;
    println!("{}", g.generate_stats().expect("failed to generate stats"));
    Ok(())
}
