use anyhow::Result;
use clap::Parser;

use rustradio::Complex;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    driver: String,
    #[arg(short, default_value = "0")]
    verbose: usize,
}

pub fn main() -> Result<()> {
    println!("soapy_fm receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = MTGraph::new();

    // Transmitter.
    let dev = soapysdr::Device::new(&*opt.driver)?;
    {
        eprintln!("Set up transmitter");
        let prev = blockchain![g, prev, ConstantSource::new(Complex::new(0.0, 0.0))];
        g.add(Box::new(
            SoapySdrSink::builder(&dev, 2_450_000_000.0, 300000.0).build(prev)?,
        ));
    }

    // Receiver.
    {
        eprintln!("Set up receiver");
        let prev = blockchain![
            g,
            prev,
            SoapySdrSource::builder(&dev, 2_450_000_000.0, 300000.0).build()?
        ];
        g.add(Box::new(NullSink::new(prev)));
    }
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");
    g.run()?;
    eprintln!("{}", g.generate_stats().expect("failed to generate stats"));
    Ok(())
}
