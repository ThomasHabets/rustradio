//! Add a constant value to every sample.
use crate::map_block_macro_v2;
use crate::stream::Stream;

/// AddConst adds a constant value to every sample.
pub struct AddConst<'a, T>
where
    T: Copy,
{
    val: T,
    src: &'a Stream<T>,
    dst: &'a Stream<T>,
}

impl<'a, T> AddConst<'a, T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Create a new AddConst, providing the constant to be added.
    pub fn new(src: &'a Stream<T>, dst: &'a Stream<T>, val: T) -> Self {
        Self { val, src, dst }
    }

    fn process_one(&self, a: &T) -> T {
        *a + self.val
    }
}
map_block_macro_v2![AddConst<'_, T>, std::ops::Add<Output = T>];
