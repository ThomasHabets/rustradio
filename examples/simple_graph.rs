use anyhow::Result;
use clap::Parser;

use rustradio::Complex;
use rustradio::blocks::{AddConst, DebugSink, PduWriter, StreamToPdu, VectorSource};
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short, default_value = "0")]
    verbose: usize,
}

fn simple_copy() -> Result<()> {
    let mut g = Graph::new();

    let (src, src_out) = VectorSource::builder(vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ])
    .repeat(rustradio::Repeat::finite(2))
    .build()?;
    let src = Box::new(src);

    let (add, add_out) = AddConst::new(src_out, Complex::new(1.1, 2.0));
    let sink = DebugSink::new(add_out);

    g.add(src);
    g.add(Box::new(add));
    g.add(Box::new(sink));

    g.run().map_err(Into::into)
}

fn simple_noncopy() -> Result<()> {
    let mut g = Graph::new();

    let (src, src_out) = VectorSource::builder(vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ])
    .repeat(rustradio::Repeat::finite(2))
    .build()?;
    let (to_pdu, prev) = StreamToPdu::new(src_out, "burst".to_string(), 10_000, 50);
    g.add(Box::new(to_pdu));
    g.add(Box::new(PduWriter::new(prev, ".")));
    //let add = Box::new(AddConst::new(src.out(), Complex::new(1.1, 2.0)));
    //let sink = Box::new(DebugSink::new(add.out()));
    //let sink = Box::new(DebugSink::new(src.out()));

    g.add(Box::new(src));
    //g.add(add);
    //g.add(sink);
    g.run().map_err(Into::into)
}

fn main() -> Result<()> {
    println!("Simple test graphs");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    simple_copy()?;
    simple_noncopy()
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example simple_graph"
 * End:
 */
