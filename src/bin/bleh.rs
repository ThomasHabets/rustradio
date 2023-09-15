use anyhow::Result;

use lib::complex_to_mag2::*;
use lib::constant_source::*;
use lib::convert::*;
use lib::debug_sink::*;
use lib::file_sink::*;
use lib::file_source::*;
use lib::multiply_const::*;
use lib::single_pole_iir_filter::*;
use lib::*;
use std::time::Instant;

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
    if false {
        bleh()?;
    }

    let mut src = FileSource::new("b200-868M-1024k-ofs-1s.c32", false)?;
    let mut mag = ComplexToMag2::new();
    let mut iir = SinglePoleIIRFilter::new(0.02).ok_or(Error::new("iir init"))?;
    let mut sink = FileSink::new("out.f32", lib::file_sink::Mode::Overwrite)?;
    let mut s1 = Stream::new(2000000);
    let mut s2 = Stream::new(2000000);
    let mut s3 = Stream::new(2000000);

    loop {
        let st = Instant::now();
        src.work(&mut s1)?;
        println!(
            "reading {} took {:?}",
            lib::StreamReader::available(&s1),
            st.elapsed()
        );
        if lib::StreamReader::available(&s1) == 0 {
            break;
        }

        let st = Instant::now();
        mag.work(&mut s1, &mut s2)?;
        println!(
            "mag {} took {:?}",
            lib::StreamReader::available(&s2),
            st.elapsed()
        );

        let st = Instant::now();
        iir.work(&mut s2, &mut s3)?;
        println!(
            "iir {} took {:?}",
            lib::StreamReader::available(&s3),
            st.elapsed()
        );

        let st = Instant::now();
        sink.work(&mut s3)?;
        println!("sink took {:?}", st.elapsed());
    }
    Ok(())
}
