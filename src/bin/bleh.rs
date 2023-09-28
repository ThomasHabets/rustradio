use anyhow::Result;

use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use rustradio::{Complex, Float};

fn main() -> Result<()> {
    println!("Hello, world!");

    {
        let s = StreamType::new_float_from_slice(&[1.0, -1.0, 3.9]);
        let mut is = InputStreams::new();
        is.add_stream(s);
        let mut add = AddConst::new(1.1);

        let s = StreamType::new_float();
        let mut os = OutputStreams::new();
        os.add_stream(s);

        add.work(&mut is, &mut os)?;
        let res: Streamp<Float> = os.get(0).into();
        println!("{:?}", &res.borrow().iter().collect::<Vec<&Float>>());
    }

    {
        let mut g = Graph::new();
        let src = g.add(Box::new(TcpSource::<Complex>::new("127.0.0.1", 2000)?));
        let add = g.add(Box::new(AddConst::new(Complex::new(1.1, 2.0))));
        let sink = g.add(Box::new(NullSink::<Complex>::new()));
        g.connect(StreamType::new_complex(), src, 0, add, 0);
        g.connect(StreamType::new_complex(), add, 0, sink, 0);
        g.run()?;
    }

    Ok(())
}
