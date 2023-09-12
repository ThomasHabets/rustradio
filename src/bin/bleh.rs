use anyhow::Result;

use lib::constant_source::*;
use lib::convert::*;
use lib::debug_sink::*;
use lib::multiply_const::*;
use lib::*;

fn main() -> Result<()> {
    println!("Hello, world!");
    let mut src = ConstantSource::new(1f32);
    let mut sink = DebugSink::new();
    let mut mul = MultiplyConst::new(2.0);
    let mut f2i = FloatToU32::new(1.0);
    let mut s1 = Stream::new(10);
    let mut s2 = Stream::new(10);
    let mut s3 = Stream::new(10);

    let wait = std::time::Duration::from_secs(1);

    loop {
        src.work(&mut s1)?;
        mul.work(&mut s1, &mut s2)?;
        f2i.work(&mut s2, &mut s3)?;
        sink.work(&mut s3)?;
        std::thread::sleep(wait);
    }
}
