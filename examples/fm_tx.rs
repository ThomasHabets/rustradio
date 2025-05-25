//! Simple analog audio FM transmitter.
//!
//! Deviation:
//! * Amateur radio: 5KHz
//! * Broadcast FM: 75KHz
//!
//! TODO:
//! * Add preemphasis.
use anyhow::Result;
use clap::Parser;
use log::warn;

use rustradio::Repeat;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(long, default_value_t = 0)]
    verbose: usize,

    /// soapysdr driver string.
    #[arg(long)]
    driver: String,

    /// Input .au file.
    #[arg(long)]
    input: std::path::PathBuf,

    /// Output gain, between 0 and 1.
    #[arg(long, default_value_t = 0.1)]
    ogain: f32,

    /// Frequency in MHz.
    #[arg(long, default_value_t = 436.2)]
    freq: f32,

    /// Audio rate.
    #[arg(long, default_value_t = 48000)]
    audio_rate: usize,

    /// Sample rate on RF side.
    #[arg(long, default_value_t = 480000)]
    sample_rate: usize,

    /// List SDR devices.
    #[arg(long)]
    list_devices: bool,

    /// FM deviation.
    #[arg(long, default_value_t = 5000)]
    deviation: usize,
}

fn main() -> Result<()> {
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    soapysdr::configure_logging();
    if opt.list_devices {
        for dev in soapysdr::enumerate("").unwrap() {
            println!("{}", dev);
        }
        return Ok(());
    }
    let mut g = MTGraph::new();
    let dev = soapysdr::Device::new(&*opt.driver)?;

    let prev = blockchain![
        g,
        prev,
        FileSource::builder(&opt.input)
            .repeat(Repeat::infinite())
            .build()?,
        AuDecode::new(prev, opt.audio_rate as u32),
        RationalResampler::<u8>::builder()
            .deci(opt.audio_rate)
            .interp(opt.sample_rate)
            .build(prev)?,
        Vco::new(
            prev,
            2.0 * std::f64::consts::PI * opt.deviation as f64 / opt.sample_rate as f64
        ),
    ];
    g.add(Box::new(
        SoapySdrSink::builder(
            &dev,
            (1_000_000.0 * opt.freq).into(),
            opt.sample_rate as f64,
        )
        .ogain(opt.ogain.into())
        .build(prev)?,
    ));

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    eprintln!("Running loop");
    g.run()?;
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}
