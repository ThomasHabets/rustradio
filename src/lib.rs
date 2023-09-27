pub mod add_const;
pub mod binary_slicer;
pub mod fft_filter;
pub mod multiply_const;

pub mod block;
pub mod blocks;
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
