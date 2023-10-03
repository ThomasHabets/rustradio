//! Multiply stream by a constant value.
use crate::map_block_macro_v2;

/// Multiply stream by a constant value.
pub struct MultiplyConst<T> {
    val: T,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    /// Create new MultiplyConst block.
    pub fn new(val: T) -> Self {
        Self { val }
    }
    fn process_one(&self, x: &T) -> T {
        *x * self.val
    }
}

map_block_macro_v2![MultiplyConst<T>, std::ops::Mul<Output = T>];
