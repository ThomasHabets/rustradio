/*!
Example tone generator via pipewire.
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
    println!("Pipewire tone generator receiver example");
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
    let prev = blockchain![
        g,
        prev,
        SignalSourceFloat::new(opt.audio_rate as Float, opt.freq, opt.volume)
    ];

    g.add(Box::new(
        PipewireSink::builder()
            .audio_rate(opt.audio_rate)
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
