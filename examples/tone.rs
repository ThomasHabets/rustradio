/*!
Example tone generator.
 */
use anyhow::Result;
use clap::Parser;
use log::warn;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::{Float, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Frequency of tone.
    #[allow(dead_code)]
    #[arg(long = "freq", default_value = "8000")]
    freq: Float,

    /// Verbosity of debug messages.
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// Tone volume.
    #[arg(long = "volume", default_value = "0.1")]
    volume: Float,

    /// Audio output rate.
    #[arg(default_value = "48000")]
    audio_rate: u32,
}

fn main() -> Result<()> {
    println!("tone generator receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = Graph::new();

    // Two ways of getting a real sine wave, just as examples.
    let prev = if false {
        blockchain![
            g,
            prev,
            SignalSourceFloat::new(opt.audio_rate as Float, opt.freq, opt.volume)
        ]
    } else {
        blockchain![
            g,
            prev,
            SignalSourceComplex::new(opt.audio_rate as Float, opt.freq, opt.volume),
            Map::keep_tags(prev, "ComplexToReal", |x| x.re),
        ]
    };

    g.add(Box::new(AudioSink::new(prev, opt.audio_rate as u64)?));

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
