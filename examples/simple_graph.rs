use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::{AddConst, DebugSink, PduWriter, StreamToPdu, VectorSourceBuilder};
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::Complex;

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "v", default_value = "0")]
    verbose: usize,
}

fn simple_copy() -> Result<()> {
    let mut g = Graph::new();

    let (src, src_out) = VectorSourceBuilder::new(vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ])
    .repeat(2)
    .build();
    let src = Box::new(src);

    let (add, add_out) = AddConst::new(src_out, Complex::new(1.1, 2.0));
    let sink = DebugSink::new(add_out);

    g.add(src);
    g.add(Box::new(add));
    g.add(Box::new(sink));

    g.run()
}

fn simple_noncopy() -> Result<()> {
    let mut g = Graph::new();

    let (src, src_out) = VectorSourceBuilder::new(vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ])
    .repeat(2)
    .build();
    let to_pdu = StreamToPdu::new(src_out, "burst".to_string(), 10_000, 50);
    let prev = to_pdu.out();
    g.add(Box::new(to_pdu));
    g.add(Box::new(PduWriter::new(prev, ".".into())));
    //let add = Box::new(AddConst::new(src.out(), Complex::new(1.1, 2.0)));
    //let sink = Box::new(DebugSink::new(add.out()));
    //let sink = Box::new(DebugSink::new(src.out()));

    g.add(Box::new(src));
    //g.add(add);
    //g.add(sink);
    g.run()
}

fn main() -> Result<()> {
    println!("Simple test graphs");
    let opt = Opt::from_args();
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
