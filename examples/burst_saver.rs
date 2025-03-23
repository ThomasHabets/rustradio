/*! Burst saver.

Listen for power bursts, and save them as separate files in an output
directory.
*/
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::{Complex, Error, Float};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(long = "out", short)]
    output: PathBuf,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "freq", default_value = "144800000")]
    freq: u64,

    #[arg(short, default_value = "0")]
    verbose: usize,

    #[arg(long = "rtlsdr")]
    rtlsdr: bool,

    #[arg(long = "sample_rate", default_value = "300000")]
    samp_rate: u32,

    #[arg(short)]
    read: Option<String>,

    #[arg(long = "threshold", default_value = "0.0001")]
    threshold: Float,

    #[arg(long = "iir_alpha", default_value = "0.01")]
    iir_alpha: Float,

    #[arg(long = "delay", default_value = "3000")]
    delay: usize,

    #[arg(long = "tail", default_value = "5000")]
    tail: usize,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let (block, prev) = $cons;
        $g.add(Box::new(block));
        prev
    }};
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

    let mut g = Graph::new();

    let (prev, samp_rate) = if let Some(read) = opt.read {
        let prev = add_block![g, FileSource::<Complex>::new(&read)?];
        (prev, opt.samp_rate as Float)
    } else if opt.rtlsdr {
        #[cfg(feature = "rtlsdr")]
        {
            // Source.
            let prev = add_block![g, RtlSdrSource::new(opt.freq, opt.samp_rate, opt.gain)?];

            // Decode.
            let prev = add_block![g, RtlSdrDecode::new(prev)];
            (prev, opt.samp_rate as Float)
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("rtlsdr feature not enabled")
    } else {
        panic!("Need to provide either --rtlsdr or -r")
    };

    // Filter RF.
    let taps = rustradio::fir::low_pass_complex(
        samp_rate,
        20_000.0,
        100.0,
        &rustradio::window::WindowType::Hamming,
    );
    let prev = add_block![g, FftFilter::new(prev, &taps)];

    // Resample RF.
    let new_samp_rate = 50_000.0;
    let prev = add_block![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    let (b, datapath, magpath) = Tee::new(prev);
    g.add(Box::new(b));
    let magpath = add_block![g, ComplexToMag2::new(magpath)];
    let magpath = add_block![
        g,
        SinglePoleIirFilter::new(magpath, opt.iir_alpha).ok_or(Error::msg("bad IIR parameters"))?
    ];
    let datapath = add_block![g, Delay::new(datapath, opt.delay)];
    let prev = add_block![
        g,
        BurstTagger::new(datapath, magpath, opt.threshold, "burst".to_string())
    ];

    let prev = add_block![
        g,
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, opt.tail)
    ];
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Set up Ctrl-C.
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    // Run.
    eprintln!("Running…");
    g.run()?;
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}
