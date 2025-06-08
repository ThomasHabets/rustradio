//! Morse code beacon.
use anyhow::Result;
use clap::Parser;

use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::parse_frequency;
use rustradio::{Complex, Float};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    driver: String,
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// TX/RX frequency.
    #[arg(long, value_parser=parse_frequency)]
    freq: f64,

    #[arg(long, value_parser=parse_frequency, default_value_t = 300000.0)]
    sample_rate: f64,

    /// Output gain. 0.0-1.0.
    #[arg(long, default_value_t = 0.0)]
    ogain: f64,

    /// Morse code speed in words per minute.
    #[arg(long, default_value_t = 20.0)]
    wpm: f32,

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
    let mut g = MTGraph::new();

    // 20 WPM is 60ms time unit.
    let raw_sps = (opt.wpm / 20.0) / 0.06;
    let prev = blockchain![
        g,
        prev,
        Strobe::new(std::time::Duration::from_secs(10), opt.msg.to_string(),),
        MorseEncode::new(prev),
        PduToStream::new(prev),
        RationalResampler::builder()
            // Multiply by 100 to get more significant digits on raw SPS, which
            // otherwise rounds 20 WPM 16.666 to 16.
            .deci((100.0 * raw_sps) as usize)
            .interp((100.0 * opt.sample_rate) as usize)
            .build(prev)?,
        Map::keep_tags(prev, "ToComplex", |s| Complex::new(s as Float, 0.0)),
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
