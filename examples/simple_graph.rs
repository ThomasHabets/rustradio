use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::{
    AddConst, DebugSink, PduWriter, StreamToPdu, VectorSource, VectorSourceBuilder,
};
use rustradio::graph::Graph;
use rustradio::stream::Stream;
use rustradio::Complex;

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "v", default_value = "0")]
    verbose: usize,
}

fn simple_copy() -> Result<()> {
    let mut g = Graph::new();

    let o = Stream::new();
    let mut src = VectorSourceBuilder::new(
        vec![
            Complex::new(10.0, 0.0),
            Complex::new(-20.0, 0.0),
            Complex::new(100.0, -100.0),
        ],
        &o,
    )
    .repeat(2)
    .build();
    let o2 = Stream::new();
    let mut add = AddConst::new(&o, &o2, Complex::new(1.1, 2.0));
    let mut sink = DebugSink::new(&o2);

    g.add(&mut src);
    g.add(&mut add);
    g.add(&mut sink);

    g.run()
}

fn simple_noncopy() -> Result<()> {
    let mut g = Graph::new();

    let o = Stream::new();
    let mut src = VectorSourceBuilder::new(
        vec![
            Complex::new(10.0, 0.0),
            Complex::new(-20.0, 0.0),
            Complex::new(100.0, -100.0),
        ],
        &o,
    )
    .repeat(2)
    .build();
    let o2 = Stream::new();
    let mut to_pdu = StreamToPdu::new(&o, &o2, "burst".to_string(), 10_000, 50);
    g.add(&mut to_pdu);
    let mut pw = PduWriter::new(&o2, ".".into());
    g.add(&mut pw);
    //let add = Box::new(AddConst::new(src.out(), Complex::new(1.1, 2.0)));
    //let sink = Box::new(DebugSink::new(add.out()));
    //let sink = Box::new(DebugSink::new(src.out()));

    g.add(&mut src);
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
    eprintln!("Copy");
    simple_copy()?;
    eprintln!("Noncopy");
    simple_noncopy()
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example simple_graph"
 * End:
 */
