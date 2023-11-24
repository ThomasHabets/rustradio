//! Multiply stream by a constant value.
use crate::map_block_macro_v2;
use crate::stream::{new_streamp, ReadStream, ReadStreamp, Streamp};

/// Multiply stream by a constant value.
pub struct MultiplyConst<T: Copy> {
    val: T,
    src: ReadStreamp<T>,
    dst: Streamp<T>,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    /// Create new MultiplyConst block.
    pub fn new(src: ReadStreamp<T>, val: T) -> Self {
        Self {
            val,
            src,
            dst: new_streamp(),
        }
    }

    fn process_one(&self, x: &T) -> T {
        *x * self.val
    }
}

map_block_macro_v2![MultiplyConst<T>, std::ops::Mul<Output = T>];
