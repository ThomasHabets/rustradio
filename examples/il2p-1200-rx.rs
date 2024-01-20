/*! IL2P 1200bps receiver.

*/
use std::path::PathBuf;

use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::{Complex, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(long = "out", short = "o", help = "Directory to write packets to")]
    _output: Option<PathBuf>,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    #[structopt(long = "sample_rate", default_value = "50000")]
    samp_rate: u32,

    #[structopt(short = "r", help = "Read I/Q from file")]
    read: String,

    #[structopt(long = "symbol_taps", default_value = "0.5,0.5", use_delimiter = true)]
    symbol_taps: Vec<Float>,

    #[structopt(long, default_value = "0.5")]
    symbol_max_deviation: Float,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = Graph::new();

    // TODO: this is a complete mess.
    let prev = add_block![g, FileSource::<Complex>::new(&opt.read, false)?];
    let samp_rate = opt.samp_rate as Float;

    // Filter RF.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 20_000.0, 100.0);
    let prev = add_block![g, FftFilter::new(prev, &taps)];

    // Resample RF.
    let new_samp_rate = 50_000.0;
    let prev = add_block![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];
    let prev = add_block![g, Hilbert::new(prev, 65)];

    // Can't use FastFM here, because it doesn't work well with
    // preemph'd input.
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    let taps = rustradio::fir::low_pass(samp_rate, 1100.0, 100.0);
    let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    let freq1 = 1200.0;
    let freq2 = 2200.0;
    let center_freq = freq1 + (freq2 - freq1) / 2.0;
    let prev = add_block![
        g,
        AddConst::new(prev, -center_freq * 2.0 * std::f32::consts::PI / samp_rate)
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
        let clock_filter = rustradio::iir_filter::IIRFilter::new(&opt.symbol_taps);
        let block = SymbolSync::new(
            prev,
            samp_rate / baud,
            opt.symbol_max_deviation,
            Box::new(rustradio::symbol_sync::TEDZeroCrossing::new()),
            Box::new(clock_filter),
        );
        let r = block.out();
        g.add(Box::new(block));
        r
    };

    let prev = add_block![g, BinarySlicer::new(prev)];
    let prev = add_block![g, XorConst::new(prev, 1)];

    // Save bits to file.
    let (a, prev) = add_block![g, Tee::new(prev)];

    let prev = add_block![
        g,
        CorrelateAccessCode::new(
            prev,
            vec![1, 1, 1, 1, 0, 0, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0],
            0,
        )
    ];

    let clock = add_block![g, ToText::new(vec![a, prev])];
    g.add(Box::new(FileSink::new(
        clock,
        "test.u8".into(),
        rustradio::file_sink::Mode::Overwrite,
    )?));

    // Run the graph.
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    // Run.
    eprintln!("Running…");
    let st = std::time::Instant::now();
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example il2p-1200-rx -- -r ../il2p-50k-1s.c32 --sample_rate 50000 -o ../packets"
 * End:
 */
