use anyhow::Result;

// Blocks.
pub mod add_const;
pub mod binary_slicer;
pub mod complex_to_mag2;
pub mod constant_source;
pub mod convert;
pub mod debug_sink;
// pub mod delay;
pub mod fft_filter;
pub mod file_sink;
pub mod file_source;
pub mod fir;
pub mod multiply_const;
pub mod null_sink;
pub mod quadrature_demod;
pub mod rational_resampler;
pub mod rtlsdr;
// pub mod single_pole_iir_filter;
pub mod symbol_sync;
pub mod tcp_source;

#[cfg(feature = "rtlsdr")]
pub mod rtlsdr_source;

pub mod block;
pub mod blocks;
pub mod graph;
pub mod stream;

pub type Float = f32;
pub type Complex = num::complex::Complex<Float>;

#[derive(Debug, Clone)]
pub struct Error {
    msg: String,
}

impl Error {
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

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Error {
        Error::new(&format!("{}", e))
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(&format!("IO error: {}", e))
    }
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
        std::mem::size_of::<Self>()
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
        std::mem::size_of::<Self>()
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Float is wrong size");
        }
        Ok(Float::from_le_bytes(data[0..Self::size()].try_into()?))
    }
    fn serialize(&self) -> Vec<u8> {
        Float::to_le_bytes(*self).to_vec()
    }
}

impl Sample for u8 {
    type Type = u8;
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: u8 is wrong size");
        }
        Ok(data[0])
    }
    fn serialize(&self) -> Vec<u8> {
        vec![*self]
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

pub mod tests {
    use super::*;
    pub fn assert_almost_equal_complex(left: &[Complex], right: &[Complex]) {
        assert_eq!(
            left.len(),
            right.len(),
            "\nleft: {:?}\nright: {:?}",
            left,
            right
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).norm_sqr().sqrt();
            if dist > 0.001 {
                assert_eq!(
                    left[i], right[i],
                    "\nElement {i}:\nleft: {:?}\nright: {:?}",
                    left, right
                );
            }
        }
    }
    pub fn assert_almost_equal_float(left: &[Float], right: &[Float]) {
        assert_eq!(
            left.len(),
            right.len(),
            "\nleft: {:?}\nright: {:?}",
            left,
            right
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).sqrt();
            if dist > 0.001 {
                assert_eq!(left[i], right[i], "\nleft: {:?}\nright: {:?}", left, right);
            }
        }
    }
}
