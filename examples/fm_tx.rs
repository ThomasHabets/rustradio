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
    #[arg(long)]
    driver: String,
    #[arg(long)]
    input: std::path::PathBuf,
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
    let mut g = MTGraph::new();
    let dev = soapysdr::Device::new(&*opt.driver)?;

    let prev = blockchain![
        g,
        prev,
        FileSource::<u8>::builder(&opt.input)
            .repeat(Repeat::infinite())
            .build()?,
        AuDecode::new(prev, 48000),
        Vco::new(prev, 1000.0),
    ];
    g.add(Box::new(
        SoapySdrSink::builder(&dev, 436_000_000.0, 48_000.0).build(prev)?,
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
