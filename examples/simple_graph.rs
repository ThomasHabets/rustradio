use anyhow::Result;
use clap::Parser;

use rustradio::Complex;
use rustradio::blocks::{AddConst, DebugSink, PduWriter, StreamToPdu, VectorSource};
use rustradio::graph::Graph;
use rustradio::graph::{GraphRunner, GraphRunnerExt};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short, value_parser = rustradio::parse_verbosity, default_value = "error")]
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

    let (add, add_out) = AddConst::new(src_out, Complex::new(1.1, 2.0))?;
    let sink = DebugSink::new(add_out);

    g.add(src);
    g.add(add);
    g.add(sink);

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
    g.add(to_pdu);
    g.add(PduWriter::new(prev, "."));

    g.add(src);
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
