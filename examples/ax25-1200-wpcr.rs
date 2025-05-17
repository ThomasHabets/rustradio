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
use rustradio::{Error, Float, blockchain};

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

    let new_samp_rate = 50_000.0;
    let prev = blockchain![
        g,
        prev,
        // Read file.
        FileSource::new(&opt.read)?,
        // Filter.
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                samp_rate,
                20_000.0,
                100.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        // Resample RF.
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?,
    ];
    let samp_rate = new_samp_rate;

    // Tee out signal strength.
    let (b, prev, burst_tee) = Tee::new(prev);
    g.add(Box::new(b));
    let burst_tee = blockchain![
        g,
        burst_tee,
        ComplexToMag2::new(burst_tee),
        SinglePoleIirFilter::new(burst_tee, opt.iir_alpha)
            .ok_or(Error::msg("bad IIR parameters"))?,
    ];

    // Save burst stream
    /*
    let (a, burst_tee) = add_block![g, Tee::new(burst_tee)];
    g.add(Box::new(FileSink::new(a, "test.f32", rustradio::file_sink::Mode::Overwrite)?));
     */

    let prev = blockchain![
        g,
        prev,
        // Demod.
        QuadratureDemod::new(prev, 1.0),
        Hilbert::new(prev, 65, &WindowType::Hamming),
        QuadratureDemod::new(prev, 1.0),
        // Filter.
        FftFilterFloat::new(
            prev,
            &rustradio::fir::low_pass(
                samp_rate,
                2400.0,
                100.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        // Tag.
        BurstTagger::new(prev, burst_tee, opt.threshold, "burst".to_string()),
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, 50),
        // Symbol sync.
        Midpointer::new(prev),
        Wpcr::builder(prev).samp_rate(opt.sample_rate).build(),
        VecToStream::new(prev),
        BinarySlicer::new(prev),
        // Delay xor, aka NRZI decode.
        NrziDecode::new(prev),
        // Decode.
        HdlcDeframer::new(prev, 10, 1500),
    ];

    // Save.
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Run.
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    g.run()?;
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-1200-wpcr -- -r ../aprs-50k.c32 -o ../packets"
 * End:
 */
