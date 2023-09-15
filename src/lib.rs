use anyhow::Result;

pub mod complex_to_mag2;
pub mod constant_source;
pub mod convert;
pub mod debug_sink;
pub mod delay;
pub mod file_sink;
pub mod file_source;
pub mod multiply_const;
pub mod quadrature_demod;
pub mod single_pole_iir_filter;
pub mod vector_sink;
pub mod vector_source;

type Float = f32;
pub type Complex = num::complex::Complex<Float>;

#[derive(Debug, Clone)]
pub struct Error {
    msg: String,
}

impl Error {
    #[allow(dead_code)] // Only used by test code.
    pub fn new(msg: &str) -> Self {
        Self {
            msg: msg.to_string(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "RustRadio Error: {}", self.msg)
    }
}

impl std::error::Error for Error {}

pub trait Source<T> {
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()>
    where
        T: Copy + Sample<Type = T> + std::fmt::Debug + Default;
}

pub trait Sink<T> {
    fn work(&mut self, r: &mut dyn StreamReader<T>) -> Result<()>
    where
        T: Copy + Sample<Type = T> + std::fmt::Debug + Default;
}

pub trait Block<Tin, Tout> {
    fn work(&mut self, r: &mut dyn StreamReader<Tin>, w: &mut dyn StreamWriter<Tout>) -> Result<()>
    where
        Tin: Copy + Sample<Type = Tin> + std::fmt::Debug + Default,
        Tout: Copy + Sample<Type = Tout> + std::fmt::Debug + Default;
}

pub trait Sample {
    type Type;
    fn size() -> usize;
    fn parse(data: &[u8]) -> Result<Self::Type>;
    fn serialize(&self) -> Vec<u8>;
}

impl Sample for Complex {
    type Type = Complex;
    fn size() -> usize {
        8
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Complex is wrong size");
        }
        let i = Float::from_le_bytes(data[0..Self::size() / 2].try_into()?);
        let q = Float::from_le_bytes(data[Self::size() / 2..].try_into()?);
        Ok(Complex::new(i, q))
    }
    fn serialize(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        ret.extend(Float::to_le_bytes(self.re));
        ret.extend(Float::to_le_bytes(self.im));
        ret
    }
}

impl Sample for Float {
    type Type = Float;
    fn size() -> usize {
        4
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Float is wrong size");
        }
        Ok(Float::from_le_bytes(data[0..Self::size()].try_into()?))
    }
    fn serialize(&self) -> Vec<u8> {
        f32::to_le_bytes(*self).to_vec()
    }
}

impl Sample for u32 {
    type Type = u32;
    fn size() -> usize {
        4
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Float is wrong size");
        }
        Ok(u32::from_le_bytes(data[0..Self::size()].try_into()?))
    }
    fn serialize(&self) -> Vec<u8> {
        u32::to_le_bytes(*self).to_vec()
    }
}

pub struct Stream<T> {
    max_samples: usize,
    data: Vec<T>,
}

pub trait StreamReader<T> {
    fn set_history(&mut self, n: usize);

    fn consume(&mut self, n: usize);
    fn buffer(&self) -> &[T];
    fn available(&self) -> usize;
}

pub trait StreamWriter<T: Copy> {
    fn available(&self) -> usize;
    fn write(&mut self, data: &[T]) -> Result<()>;
}

impl<T> StreamReader<T> for Stream<T> {
    fn set_history(&mut self, _n: usize) {
        todo!();
    }
    fn consume(&mut self, n: usize) {
        self.data.drain(0..n);
    }
    fn buffer(&self) -> &[T] {
        &self.data
    }
    fn available(&self) -> usize {
        self.data.len()
    }
}

impl<T: Copy> StreamWriter<T> for Stream<T> {
    fn write(&mut self, data: &[T]) -> Result<()> {
        //println!("Writing {} samples", data.len());
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
