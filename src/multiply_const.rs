//! Multiply stream by a constant value.
use crate::map_block_macro_v2;
use crate::stream::{new_streamp2, Streamp2};

/// Multiply stream by a constant value.
pub struct MultiplyConst<T: Copy> {
    val: T,
    src: Streamp2<T>,
    dst: Streamp2<T>,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    /// Create new MultiplyConst block.
    pub fn new(src: Streamp2<T>, val: T) -> Self {
        Self {
            val,
            src,
            dst: new_streamp2(),
        }
    }

    fn process_one(&self, x: &T) -> T {
        *x * self.val
    }
}

map_block_macro_v2![MultiplyConst<T>, std::ops::Mul<Output = T>];
