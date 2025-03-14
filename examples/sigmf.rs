use anyhow::Result;
use clap::Parser;

use rustradio::Float;
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short, default_value = "0")]
    verbose: usize,

    #[arg(long = "sample_rate", default_value = "50000")]
    samp_rate: f64,

    #[arg(short, help = "Read I/Q from file")]
    read: String,
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
    let prev = add_block![
        g,
        SigMFSourceBuilder::<num_complex::Complex<i32>>::new(opt.read.clone())
            .sample_rate(opt.samp_rate)
            .build()?
    ];
    let prev = add_block![
        g,
        MapBuilder::new(prev, |x| {
            num_complex::Complex::new(x.re as Float, x.im as Float)
        })
        .build()
    ];
    let (dbg, prev) = DebugFilter::new(prev);
    g.add(Box::new(dbg));
    g.add(Box::new(NoCopyFileSink::new(
        prev,
        "out.txt".into(),
        rustradio::file_sink::Mode::Overwrite,
    )?));
    g.run()?;
    Ok(())
}
