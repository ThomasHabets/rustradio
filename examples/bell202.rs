use anyhow::Result;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::Complex;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "d")]
    driver: String,
    #[structopt(short = "v", default_value = "0")]
    verbose: usize,
}
macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
}
pub fn main() -> Result<()> {
    println!("soapy_fm receiver example");
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = Graph::new();
    // Transmitter.
    {
        let prev = add_block![g, ConstantSource::new(Complex::new(0.0, 0.0))];
        g.add(Box::new(
            SoapySdrSinkBuilder::new(opt.driver.clone(), 100000000.0, 300000.0).build(prev)?,
        ));
    }

    // Receiver.
    if false {
        let prev = add_block![
            g,
            SoapySdrSourceBuilder::new(opt.driver, 10000000.0, 300000.0).build()?
        ];
        g.add(Box::new(NullSink::new(prev)));
    }
    g.run()?;
    Ok(())
}
