use anyhow::Result;
type Complex = num::complex::Complex<f32>;

struct StreamWriter<T> {
    max_samples: usize,
    data: Vec<T>,
}

impl<T: Copy> StreamWriter<T> {
    fn new(max_samples: usize) -> Self {
        Self{
            max_samples,
            data: Vec::new(),
        }
    }
    fn write(&mut self, data: &[T]) -> Result<()> {
        println!("Writing {} samples", data.len());
        self.data.extend_from_slice(data);
        Ok(())
    }
    fn available(&self) -> usize {
        self.max_samples - self.data.len()
    }
    fn consume(&mut self, n: usize) {
        self.data.drain(0..n);
    }
}

trait Sample {
    type Type;
    fn size() -> usize;
    fn parse(data: &[u8]) -> Result<Self::Type>;
}

impl Sample for Complex {
    type Type = Complex;
    fn size() -> usize {8}
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}

impl Sample for f32 {
    type Type = f32;
    fn size() -> usize {4}
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}

struct ConstantSource<T> {
    val: T,
}

impl<T: Copy + Sample<Type=T> + std::fmt::Debug> ConstantSource<T> {
    fn new(val: T) -> Self {
        Self{val}
    }
    fn work(&mut self, w: &mut StreamWriter<T>) -> Result<()> {
        w.write(&vec![self.val; w.available()])
    }
}

struct DebugSink {}
impl DebugSink {
    fn new() -> Self {
        Self{}
    }
    fn work<T: Copy + Sample<Type=T> + std::fmt::Debug>(&mut self, w: &mut StreamWriter<T>) -> Result<()> {
        for d in w.data.clone().into_iter() {
            println!("debug: {:?}", d);
        }
        w.consume(w.data.len());
        Ok(())
    }
}
struct MultiplyConst<T> {
    val: T,
}

impl<T> MultiplyConst<T>
    where T: Copy + Sample<Type=T> + std::fmt::Debug + std::ops::Mul<Output=T>
{
    fn new(val: T) -> Self {
        Self{val}
    }
    fn work(&mut self, r: &mut StreamWriter<T>, w: &mut StreamWriter<T>) -> Result<()> {
        let mut v: Vec<T> = Vec::new();
        for d in r.data.clone().into_iter() {
            v.push(d * self.val);
        }
        w.write(v.as_slice());
        r.consume(v.len());
        Ok(())
    }
}

fn main() -> Result<()> {
    println!("Hello, world!");
    let mut src = ConstantSource::new(1f32);
    let mut sink = DebugSink::new();
    let mut mul = MultiplyConst::new(2.0);
    let mut s1 = StreamWriter::new(10);
    let mut s2 = StreamWriter::new(10);
    loop {
        src.work(&mut s1)?;
        mul.work(&mut s1, &mut s2)?;
        sink.work(&mut s2)?;
        break;
    }
    Ok(())
}
