//! RTL-SDR Source that just reads, downsamples, and prints to stdout.
//!
//! For use with `ws_stdout` to give data to WASM via websocket.
use anyhow::Result;
use clap::Parser;

use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Tuned frequency, if reading from RTL SDR.
    #[allow(dead_code)]
    #[arg(long = "freq", default_value_t = 100_000_000)]
    freq: u64,

    /// Verbosity of debug messages.
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// Input gain, if reading from RTL SDR.
    #[allow(dead_code)]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,
}

fn run(opt: Opt) -> Result<()> {
    let samp_rate = 250_000;
    let samp_rate_2 = 50_000;
    let mut g = Graph::new();
    let prev = blockchain![
        g,
        prev,
        RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?,
        RtlSdrDecode::new(prev),
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                samp_rate as f32,
                40_000.0,
                1_000.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::builder()
            .deci(samp_rate as usize)
            .interp(samp_rate_2 as usize)
            .build(prev)?,
        RtlSdrEncode::new(prev),
    ];
    let sink = Box::new(
        FileSink::builder("/dev/stdout")
            .mode(Mode::Overwrite)
            .build(prev)?,
    );
    g.add(sink);
    Ok(g.run()?)
}

fn main() -> Result<()> {
    eprintln!("rtl_fm receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    run(opt)
}
