use anyhow::Result;
use clap::Parser;

use rustradio::Complex;
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    driver: String,
    #[arg(short, default_value = "0")]
    verbose: usize,
}
macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let (block, prev) = $cons;
        $g.add(Box::new(block));
        prev
    }};
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

    let mut g = Graph::new();

    // Transmitter.
    let dev = soapysdr::Device::new(&*opt.driver)?;
    {
        eprintln!("Set up transmitter");
        let prev = add_block![g, ConstantSource::new(Complex::new(0.0, 0.0))];
        g.add(Box::new(
            SoapySdrSink::builder(&dev, 2_450_000_000.0, 300000.0).build(prev)?,
        ));
    }

    // Receiver.
    if true {
        eprintln!("Set up receiver");
        let prev = add_block![
            g,
            SoapySdrSource::builder(&dev, 2_450_000_000.0, 300000.0).build()?
        ];
        g.add(Box::new(NullSink::new(prev)));
    }
    g.run()?;
    Ok(())
}
