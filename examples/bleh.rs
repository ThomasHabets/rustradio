use anyhow::Result;

use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::Stream;
use rustradio::Float;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    println!("Running some block without Graph");
    {
        let mut v: Vec<&mut dyn Block> = Vec::new();
        let mut src = VectorSource::new(vec![1.0, -1.0, 3.21], true);
        let mut add = AddConst::new(src.out(), 1.1);
        v.push(&mut src);

        let mut debug = DebugSink::new(add.out());
        v.push(&mut add);
        v.push(&mut debug);

        loop {
            for b in &mut v {
                b.work()?;
            }
        }
    }

    #[cfg(feature = "rtlsdr")]
    {
        println!("Running rtlsdr example");
        use rustradio::graph::Graph;
        use rustradio::Complex;
        //let mut g = Graph::new();
        let mut src = RtlSdrSource::new(868_000_000, 1024_000, 30)?;
        let mut dec = RtlSdrDecode::new(src.out());
        let mut add = AddConst::new(dec.out(), Complex::new(1.1, 2.0));
        let mut sink = NullSink::new(add.out());
        let mut v: Vec<&mut dyn Block> = vec![&mut src, &mut dec, &mut add, &mut sink];
        loop {
            for b in &mut v {
                b.work()?;
            }
        }
    }

    Ok(())
}
