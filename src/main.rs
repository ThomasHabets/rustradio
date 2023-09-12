use anyhow::Result;
type Float = f32;
type Complex = num::complex::Complex<Float>;

struct Stream<T> {
    max_samples: usize,
    data: Vec<T>,
}

trait StreamReader<T> {
    fn consume(&mut self, n: usize);
    fn buffer(&self) -> &[T];
}

trait StreamWriter<T: Copy> {
    fn available(&self) -> usize;
    fn write(&mut self, data: &[T]) -> Result<()>;
}

impl<T> StreamReader<T> for Stream<T> {
    fn consume(&mut self, n: usize) {
        self.data.drain(0..n);
    }
    fn buffer(&self) -> &[T] {
        &self.data
    }
}

impl<T: Copy> StreamWriter<T> for Stream<T> {
    fn write(&mut self, data: &[T]) -> Result<()> {
        println!("Writing {} samples", data.len());
        self.data.extend_from_slice(data);
        Ok(())
    }
    fn available(&self) -> usize {
        self.max_samples - self.data.len()
    }
}

impl<T> Stream<T> {
    fn new(max_samples: usize) -> Self {
        Self {
            max_samples,
            data: Vec::new(),
        }
    }
}

trait Sample {
    type Type;
    fn size() -> usize;
    fn parse(data: &[u8]) -> Result<Self::Type>;
}

impl Sample for Complex {
    type Type = Complex;
    fn size() -> usize {
        8
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}

impl Sample for Float {
    type Type = Float;
    fn size() -> usize {
        4
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}
impl Sample for u32 {
    type Type = u32;
    fn size() -> usize {
        4
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        todo!();
    }
}

struct ConstantSource<T> {
    val: T,
}

impl<T: Copy + Sample<Type = T> + std::fmt::Debug> ConstantSource<T> {
    fn new(val: T) -> Self {
        Self { val }
    }
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        w.write(&vec![self.val; w.available()])
    }
}

struct DebugSink {}
impl DebugSink {
    fn new() -> Self {
        Self {}
    }
    fn work<T: Copy + Sample<Type = T> + std::fmt::Debug>(
        &mut self,
        r: &mut dyn StreamReader<T>,
    ) -> Result<()> {
        for d in r.buffer().clone().into_iter() {
            println!("debug: {:?}", d);
        }
        r.consume(r.buffer().len());
        Ok(())
    }
}
struct MultiplyConst<T> {
    val: T,
}

impl<T> MultiplyConst<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + std::ops::Mul<Output = T>,
{
    fn new(val: T) -> Self {
        Self { val }
    }
    fn work(&mut self, r: &mut dyn StreamReader<T>, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let mut v: Vec<T> = Vec::new();
        for d in r.buffer().clone().into_iter() {
            v.push(*d * self.val);
        }
        w.write(v.as_slice())?;
        r.consume(v.len());
        Ok(())
    }
}

/*
struct Convert<From, To> {
    scale_from: From,
    scale_to: To,
}
impl std::convert::Into<u32> for Float {
    fn into(t: Float) -> u32 {
        t as u32
    }
}
impl<From, To> Convert<From, To>
where From: std::ops::Mul<Output=From> + std::convert::TryInto<To>,
      To: std::ops::Mul<Output=To>
{
    fn new(scale_from: From, scale_to: To) -> Self {
        Self{
            scale_from,
            scale_to,
        }
    }
    fn work(&mut self, r: &mut Stream<From>, w: &mut Stream<To>) -> Result<()>
    where <From as TryInto<To>>::Error: std::fmt::Debug
    {
        let v = r.data.iter().map(|e| {
            //From::into(*e * self.scale_from) * self.scale_to
            (*e * self.scale_from).try_into().unwrap() * self.scale_to
        });
        Ok(())
    }
}
*/
struct FloatToU32 {
    scale: Float,
}
impl FloatToU32 {
    fn new(scale: Float) -> Self {
        Self { scale }
    }
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Float>,
        w: &mut dyn StreamWriter<u32>,
    ) -> Result<()> {
        let v: Vec<u32> = r
            .buffer()
            .iter()
            .map(|e| (*e * self.scale) as u32)
            .collect();
        w.write(&v)
    }
}

fn main() -> Result<()> {
    println!("Hello, world!");
    let mut src = ConstantSource::new(1f32);
    let mut sink = DebugSink::new();
    let mut mul = MultiplyConst::new(2.0);
    let mut f2i = FloatToU32::new(1.0);
    let mut s1 = Stream::new(10);
    let mut s2 = Stream::new(10);
    let mut s3 = Stream::new(10);
    loop {
        src.work(&mut s1)?;
        mul.work(&mut s1, &mut s2)?;
        f2i.work(&mut s2, &mut s3)?;
        sink.work(&mut s3)?;
        break;
    }
    Ok(())
}
