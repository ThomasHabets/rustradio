use anyhow::Result;

pub mod constant_source;
pub mod convert;
pub mod debug_sink;
pub mod multiply_const;

type Float = f32;
type Complex = num::complex::Complex<Float>;

pub trait Sample {
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

pub struct Stream<T> {
    max_samples: usize,
    data: Vec<T>,
}

pub trait StreamReader<T> {
    fn consume(&mut self, n: usize);
    fn buffer(&self) -> &[T];
}

pub trait StreamWriter<T: Copy> {
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
    pub fn new(max_samples: usize) -> Self {
        Self {
            max_samples,
            data: Vec::new(),
        }
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
    pub fn work(&mut self, r: &mut Stream<From>, w: &mut Stream<To>) -> Result<()>
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
