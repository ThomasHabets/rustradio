use anyhow::Result;
use std::cell::RefCell;
use std::rc::Rc;

use lib::complex_to_mag2::*;
use lib::constant_source::*;
use lib::convert::*;
use lib::debug_sink::*;
use lib::file_sink::*;
use lib::file_source::*;
use lib::fir::FIRFilter;
use lib::multiply_const::*;
use lib::quadrature_demod::*;
use lib::rational_resampler::*;
use lib::single_pole_iir_filter::*;
use lib::*;
use std::time::Instant;

fn bleh() -> Result<()> {
    let mut src = ConstantSource::new(1.0 as Float);
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

struct Graph {
    work: Vec<Box<dyn FnMut() -> Result<usize>>>,
}

impl Graph {
    fn new() -> Self {
        Self { work: Vec::new() }
    }
    fn add_source<T>(&mut self, mut block: Box<dyn Source<T>>) -> Rc<RefCell<Stream<T>>>
    where
        T: Copy + std::fmt::Debug + Sample<Type = T> + Default + 'static,
    {
        let stream = Rc::new(RefCell::new(Stream::new(819200)));
        let ws = Rc::clone(&stream);
        self.work.push(Box::new(move || {
            block.work(&mut *ws.borrow_mut())?;
            Ok(ws.borrow().available())
        }));
        stream
    }
    fn add_block<Tin, Tout>(
        &mut self,
        input: Rc<RefCell<Stream<Tin>>>,
        mut block: Box<dyn Block<Tin, Tout>>,
    ) -> Rc<RefCell<Stream<Tout>>>
    where
        Tin: Copy + std::fmt::Debug + Sample<Type = Tin> + Default + 'static,
        Tout: Copy + std::fmt::Debug + Sample<Type = Tout> + Default + 'static,
    {
        let stream = Rc::new(RefCell::new(Stream::new(819200)));
        let ws = Rc::clone(&stream);
        self.work.push(Box::new(move || {
            block.work(&mut *input.borrow_mut(), &mut *ws.borrow_mut())?;
            Ok(ws.borrow().available())
        }));
        stream
    }
    fn add_sink<T>(&mut self, input: Rc<RefCell<Stream<T>>>, mut block: Box<dyn Sink<T>>)
    where
        T: Copy + std::fmt::Debug + Sample<Type = T> + Default + 'static,
    {
        self.work.push(Box::new(move || {
            block.work(&mut *input.borrow_mut())?;
            Ok(0)
        }));
    }
    fn work(&mut self) -> Result<usize> {
        let mut total = 0;
        for w in &mut self.work {
            total += w()?;
        }
        Ok(total)
    }
}

fn bleh2() -> Result<()> {
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

fn main() -> Result<()> {
    println!("Hello, world!");
    if false {
        bleh()?;
    }
    if false {
        bleh2()?;
    }

    if false {
        let src = FileSource::new("b200-868M-1024k-ofs-1s.c32", false)?;
        let mut g = Graph::new();
        let s = g.add_source::<Complex>(Box::new(src));
        let s = g.add_block(s, Box::new(ComplexToMag2::new()));
        let s = g.add_block(
            s,
            Box::new(SinglePoleIIRFilter::new(0.02).ok_or(Error::new("IIR init"))?),
        );
        g.add_sink(
            s,
            Box::new(FileSink::new("out.f32", lib::file_sink::Mode::Overwrite)?),
        );
        loop {
            println!("Working…");
            if g.work()? == 0 {
                break;
            }
            //std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    if true {
        let src = FileSource::new("b200-868M-1024k-ofs-1s.c32", false)?;
        let samp_rate = 1024000.0;
        let mut g = Graph::new();
        let s = g.add_source::<Complex>(Box::new(src));
        let taps = lib::fir::low_pass(samp_rate, 50000.0, 1000.0);
        let s = g.add_block(s, Box::new(FIRFilter::new(taps.as_slice())));
        let new_samp_rate = 200000.0;
        let s = g.add_block(
            s,
            Box::new(RationalResampler::new(
                new_samp_rate as usize,
                samp_rate as usize,
            )?),
        );
        let _samp_rate = new_samp_rate;

        let s = g.add_block(s, Box::new(QuadratureDemod::new(1.0)));
        // TODO: symbol sync
        // TODO: binary slicer
        // TODO: CAC
        g.add_sink(
            s,
            Box::new(FileSink::new("out.f32", lib::file_sink::Mode::Overwrite)?),
        );
        loop {
            let n = g.work()?;
            if n <= 1 {
                break;
            }
            println!("Got {n}…");
            //std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    Ok(())
}
