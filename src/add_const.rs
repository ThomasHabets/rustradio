//! Add a constant value to every sample.
use crate::map_block_macro_v2;
use crate::stream::{new_streamp, Streamp, ReadStreamp};

/// AddConst adds a constant value to every sample.
pub struct AddConst<T>
where
    T: Copy,
{
    val: T,
    src: ReadStreamp<T>,
    dst: Streamp<T>,
}

impl<T> AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Create a new AddConst, providing the constant to be added.
    pub fn new(src: ReadStreamp<T>, val: T) -> Self {
        Self {
            val,
            src,
            dst: new_streamp(),
        }
    }

    fn process_one(&self, a: &T) -> T {
        *a + self.val
    }
}
map_block_macro_v2![AddConst<T>, std::ops::Add<Output = T>];
