//! KISS client that only outputs business information sent by the peer.
//!
//! The use for this is the equivalent of the TX/RX LEDs of physical modems.
use anyhow::Result;
use clap::Parser;

use rustradio::blockchain;
use rustradio::blocks::{DebugSink, KissDecode, KissFrame, NCNullSink, TcpSource};
use rustradio::graph::{GraphBuildExt, GraphRunner};

#[derive(clap::Parser)]
struct Opt {
    connect: String,

    /// Log level: error, warn, info, debug, or trace.
    #[arg(short, default_value = "error", value_parser = rustradio::parse_verbosity)]
    verbose: usize,
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

    let mut graph = rustradio::graph::Graph::new();
    let prev = blockchain![
        graph,
        prev,
        TcpSource::<u8>::new(opt.connect.as_str())?,
        KissFrame::new(prev),
        {
            let (mut b, prev) = KissDecode::new(prev);
            let null = NCNullSink::new(prev);
            graph.add(null);
            let prev = b.busy()?;
            (b, prev)
        },
    ];
    graph.add(DebugSink::new(prev));
    eprintln!("Running…");
    graph.run()?;
    Ok(())
}
