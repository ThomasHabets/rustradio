use anyhow::Result;

use lib::complex_to_mag2::*;
use lib::constant_source::*;
use lib::convert::*;
use lib::debug_sink::*;
use lib::file_sink::*;
use lib::file_source::*;
use lib::multiply_const::*;
use lib::*;

fn bleh() -> Result<()> {
    let mut src = ConstantSource::new(1f32);
    let mut sink: Box<dyn Sink<u32>> = {
        if false {
            Box::new(DebugSink::<u32>::new())
        } else {
            Box::new(FileSink::new("out.bin", lib::file_sink::Mode::Overwrite)?)
        }
    };
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

fn main() -> Result<()> {
    println!("Hello, world!");

    let mut src = FileSource::new("b200-868M-1024k-ofs-1s.c32", false)?;
    let mut mag = ComplexToMag2::new();
    let mut sink = FileSink::new("out.f32", lib::file_sink::Mode::Overwrite)?;
    let mut s1 = Stream::new(1000000);
    let mut s2 = Stream::new(1000000);

    loop {
        eprintln!(">>> src");
        src.work(&mut s1);
        println!("data left in s1 {}", lib::StreamReader::available(&s1));

        eprintln!(">>> mag");
        mag.work(&mut s1, &mut s2);
        if lib::StreamReader::available(&s2) == 0 {
            break;
        }
        println!("about to print {}", lib::StreamReader::available(&s2));

        eprintln!(">>> sink");
        sink.work(&mut s2);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(())
}
