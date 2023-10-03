use anyhow::Result;

use rustradio::blocks::{AddConst, DebugSink, VectorSource};
use rustradio::graph::Graph;
use rustradio::stream::StreamType;
use rustradio::Complex;

fn main() -> Result<()> {
    let mut g = Graph::new();
    let src = g.add(Box::new(VectorSource::new(
        vec![
            Complex::new(10.0, 0.0),
            Complex::new(-20.0, 0.0),
            Complex::new(100.0, -100.0),
        ],
        false,
    )));
    let add = g.add(Box::new(AddConst::new(Complex::new(1.1, 2.0))));
    let sink = g.add(Box::new(DebugSink::<Complex>::new()));
    g.connect(StreamType::new_complex(), src, 0, add, 0);
    g.connect(StreamType::new_complex(), add, 0, sink, 0);
    g.run()
}
