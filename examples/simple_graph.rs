use anyhow::Result;

use rustradio::blocks::{AddConst, DebugSink, VectorSource};
use rustradio::graph::Graph;
use rustradio::Complex;

fn main() -> Result<()> {
    let mut g = Graph::new();

    let src = Box::new(VectorSource::new(
        vec![
            Complex::new(10.0, 0.0),
            Complex::new(-20.0, 0.0),
            Complex::new(100.0, -100.0),
        ],
        false,
    ));
    let add = Box::new(AddConst::new(src.out(), Complex::new(1.1, 2.0)));
    let sink = Box::new(DebugSink::new(add.out()));

    g.add(src);
    g.add(add);
    g.add(sink);

    g.run()
}
