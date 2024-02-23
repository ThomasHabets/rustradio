use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::Complex;

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    #[structopt(long = "sample_rate", default_value = "50000")]
    samp_rate: f64,

    #[structopt(short = "r", help = "Read I/Q from file")]
    read: String,
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
    let _prev = add_block![g, SigMFSource::<Complex>::new(&opt.read, opt.samp_rate)?];
    Ok(())
}
