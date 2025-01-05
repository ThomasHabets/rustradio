use anyhow::Result;

use rustradio::block::Block;
use rustradio::blocks::*;

fn main() -> Result<()> {
    println!("Running some block without Graph");
    if true {
        let mut v: Vec<Box<dyn Block>> = Vec::new();
        let (src, src_out) = VectorSource::new(vec![1.0, -1.0, 3.21]);
        let (add, add_out) = AddConst::new(src_out, 1.1);
        v.push(Box::new(src));

        let debug = DebugSink::new(add_out);
        v.push(Box::new(add));
        v.push(Box::new(debug));

        loop {
            for b in &mut v {
                b.work()?;
            }
        }
    }

    #[cfg(feature = "rtlsdr")]
    {
        println!("Running rtlsdr example");
        //use rustradio::graph::Graph;
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
    #[allow(unreachable_code)]
    Ok(())
}
