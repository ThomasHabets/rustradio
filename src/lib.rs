pub mod add_const;
pub mod block;
pub mod blocks;
pub mod multiply_const;
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
