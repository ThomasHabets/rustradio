//! Multiply stream by a constant value.
use crate::map_block_macro_v2;
use crate::stream::{Stream, Streamp};

/// Multiply stream by a constant value.
pub struct MultiplyConst<T: Copy> {
    val: T,
    src: Streamp<T>,
    dst: Streamp<T>,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    /// Create new MultiplyConst block.
    pub fn new(src: Streamp<T>, val: T) -> Self {
        Self {
            val,
            src,
            dst: Stream::newp(),
        }
    }

    fn process_one(&self, x: &T) -> T {
        *x * self.val
    }
}

map_block_macro_v2![MultiplyConst<T>, std::ops::Mul<Output = T>];
