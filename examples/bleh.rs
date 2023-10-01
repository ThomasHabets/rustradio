use anyhow::Result;

use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use rustradio::Float;

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

    #[cfg(features = "rtlsdr")]
    {
        use rustradio::graph::Graph;
        use rustradio::Complex;
        let mut g = Graph::new();
        let src = g.add(Box::new(RtlSdrSource::new(868_000_000, 1024_000, 30)?));
        let dec = g.add(Box::new(RtlSdrDecode::new()));
        let add = g.add(Box::new(AddConst::new(Complex::new(1.1, 2.0))));
        let sink = g.add(Box::new(NullSink::<Complex>::new()));
        g.connect(StreamType::new_u8(), src, 0, dec, 0);
        g.connect(StreamType::new_complex(), dec, 0, add, 0);
        g.connect(StreamType::new_complex(), add, 0, sink, 0);
        g.run()?;
    }

    Ok(())
}
