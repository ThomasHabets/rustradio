/*! Test program for whole packet clock recovery.

This is the same as ax25-1200-rx.rs, except it has fewer options
(e.g. only supports reading from a file), and uses WPCR instead of
ZeroCrossing symbol sync.

Ideally this should be tested using [the standard test CD][cd], but we
need the raw I/Q for burst detection. Just the audio won't do.

[cd]: http://wa8lmf.net/TNCtest/index.htm
 */
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use rustradio::blocks::*;
use rustradio::window::WindowType;
use rustradio::{Error, Float};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short = 'r')]
    read: String,

    #[arg(long = "sample_rate", default_value = "50000")]
    sample_rate: Float,

    #[arg(short, long = "out")]
    output: PathBuf,

    #[arg(short, default_value = "0")]
    verbose: usize,

    #[arg(long = "threshold", default_value = "0.0001")]
    threshold: Float,

    #[arg(long = "iir_alpha", default_value = "0.01")]
    iir_alpha: Float,

    #[arg(long)]
    multithreaded: bool,
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
        .init()
        .unwrap();

    let samp_rate = opt.sample_rate;
    let mut g: Box<dyn rustradio::graph::GraphRunner> = if opt.multithreaded {
        Box::new(rustradio::mtgraph::MTGraph::new())
    } else {
        Box::new(rustradio::graph::Graph::new())
    };

    // Read file.
    let prev = add_block![g, FileSource::new(&opt.read, false)?];

    // Filter.
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

    // Tee out signal strength.
    let (b, prev, burst_tee) = Tee::new(prev);
    g.add(Box::new(b));
    let burst_tee = add_block![g, ComplexToMag2::new(burst_tee)];
    let burst_tee = add_block![
        g,
        SinglePoleIIRFilter::new(burst_tee, opt.iir_alpha)
            .ok_or(Error::new("bad IIR parameters"))?
    ];

    // Save burst stream
    /*
    let (a, burst_tee) = add_block![g, Tee::new(burst_tee)];
    g.add(Box::new(FileSink::new(a, "test.f32", rustradio::file_sink::Mode::Overwrite)?));
     */

    // Demod.
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];
    let prev = add_block![g, Hilbert::new(prev, 65, &WindowType::Hamming)];
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    // Filter.
    let taps = rustradio::fir::low_pass(
        samp_rate,
        2400.0,
        100.0,
        &rustradio::window::WindowType::Hamming,
    );
    let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    // Tag.
    let prev = add_block![
        g,
        BurstTagger::new(prev, burst_tee, opt.threshold, "burst".to_string())
    ];

    let prev = add_block![
        g,
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, 50)
    ];

    // Symbol sync.
    let prev = add_block![g, Midpointer::new(prev)];
    let prev = add_block![g, WpcrBuilder::new(prev).samp_rate(opt.sample_rate).build()];
    let prev = add_block![g, VecToStream::new(prev)];
    let prev = add_block![g, BinarySlicer::new(prev)];

    // Delay xor, aka NRZI decode.
    let prev = add_block![g, NrziDecode::new(prev)];

    // Decode.
    let prev = add_block![g, HdlcDeframer::new(prev, 10, 1500)];

    // Save.
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Run.
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    let st = std::time::Instant::now();
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-1200-wpcr -- -r ../aprs-50k.c32 -o ../packets"
 * End:
 */
