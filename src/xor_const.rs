//! Xor a constant value with every sample.
use crate::map_block_macro_v2;
use crate::stream::{new_streamp2, Streamp2};

/// XorConst xors a constant value to every sample.
pub struct XorConst<T>
where
    T: Copy,
{
    val: T,
    src: Streamp2<T>,
    dst: Streamp2<T>,
}

impl<T> XorConst<T>
where
    T: Copy + std::ops::BitXor<Output = T>,
{
    /// Create a new XorConst, providing the constant to be xored.
    pub fn new(src: Streamp2<T>, val: T) -> Self {
        Self {
            val,
            src,
            dst: new_streamp2(),
        }
    }

    fn process_one(&self, a: &T) -> T {
        *a ^ self.val
    }
}
map_block_macro_v2![XorConst<T>, std::ops::BitXor<Output = T>];
