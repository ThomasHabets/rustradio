/*! IL2P 1200bps receiver.

Some test data for descrambler:

let ax25 = vec![0x96, 0x82, 0x64, 0x88, 0x8a, 0xae, 0xe4, 0x96, 0x96, 0x68, 0x90, 0x8a, 0x94, 0x6f, 0xb1];
let il2p = vec![0x2b, 0xa1, 0x12, 0x24, 0x25, 0x77, 0x6b, 0x2b, 0x54, 0x68, 0x25, 0x2a, 0x27];
let scrambled = vec![0x26, 0x57, 0x4d, 0x57, 0xf1, 0x96, 0xcc, 0x85, 0x42, 0xe7, 0x24, 0xf7, 0x2e, 0x8a, 0x97];

*/
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::window::WindowType;
use rustradio::{Float, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(long = "out", short, help = "Directory to write packets to")]
    _output: Option<PathBuf>,

    #[arg(short, default_value = "0")]
    verbose: usize,

    #[arg(long = "sample_rate", default_value = "50000")]
    samp_rate: u32,

    #[arg(short, help = "Read I/Q from file")]
    read: String,

    #[arg(
        long = "symbol_taps",
        default_value = "0.5,0.5",
        use_value_delimiter = true
    )]
    symbol_taps: Vec<Float>,

    #[arg(long, default_value = "0.5")]
    symbol_max_deviation: Float,
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

    // TODO: this is a complete mess.
    let samp_rate = opt.samp_rate as Float;
    let new_samp_rate = 50_000.0;
    let freq1 = 1200.0;
    let freq2 = 2200.0;
    let center_freq = freq1 + (freq2 - freq1) / 2.0;

    let prev = blockchain![
        g,
        prev,
        FileSource::new(&opt.read)?,
        // Filter RF.
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(samp_rate, 20_000.0, 100.0, &WindowType::Hamming)
        ),
        // Resample RF.
        RationalResampler::builder()
            .deci(samp_rate as usize)
            .interp(new_samp_rate as usize)
            .build(prev)?,
        QuadratureDemod::new(prev, 1.0),
        Hilbert::new(prev, 65, &WindowType::Hamming),
        // Can't use FastFM here, because it doesn't work well with
        // preemph'd input.
        QuadratureDemod::new(prev, 1.0),
        FftFilterFloat::new(
            prev,
            &rustradio::fir::low_pass(samp_rate, 1100.0, 100.0, &WindowType::Hamming)
        ),
        AddConst::new(prev, -center_freq * 2.0 * std::f32::consts::PI / samp_rate),
    ];

    /*
    // Save floats to file.
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "test.f32",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */
    let baud = 1200.0;
    let prev = {
        let clock_filter = rustradio::iir_filter::IirFilter::new(&opt.symbol_taps);
        let (block, prev) = SymbolSync::new(
            prev,
            samp_rate / baud,
            opt.symbol_max_deviation,
            Box::new(rustradio::symbol_sync::TedZeroCrossing::new()),
            Box::new(clock_filter),
        );
        g.add(Box::new(block));
        prev
    };

    let prev = blockchain![
        g,
        prev,
        BinarySlicer::new(prev),
        XorConst::new(prev, 1),
        CorrelateAccessCodeTag::new(
            prev,
            rustradio::il2p_deframer::SYNC_WORD.to_vec(),
            "sync",
            0,
        ),
    ];

    let (b, a, prev) = Tee::new(prev);
    g.add(Box::new(b));
    let clock = blockchain![g, clock, ToText::new(vec![a])];
    g.add(Box::new(FileSink::new(
        clock,
        "test.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));

    let _ = blockchain![g, prev, Il2pDeframer::new(prev)];
    //g.add(Box::new(NullSink::new(prev)));

    // Run the graph.
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
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example il2p-1200-rx -- -r ../il2p-50k-1s.c32 --sample_rate 50000 -o ../packets"
 * End:
 */
