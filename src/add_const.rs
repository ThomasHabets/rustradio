//! Add a constant value to every sample.
use crate::map_block_macro_v2;
use crate::stream::{Stream, Streamp};

/// Add const value, implemented in terms of Map.
/// TODO: remove AddConst, below?
pub fn add_const<T>(src: Streamp<T>, val: T) -> crate::convert::Map<T, T, impl Fn(T) -> T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    crate::convert::MapBuilder::new(src, move |x| x + val)
        .name("add_const".into())
        .build()
}

/// AddConst adds a constant value to every sample.
pub struct AddConst<T>
where
    T: Copy,
{
    val: T,
    src: Streamp<T>,
    dst: Streamp<T>,
}

impl<T> AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Create a new AddConst, providing the constant to be added.
    pub fn new(src: Streamp<T>, val: T) -> Self {
        Self {
            val,
            src,
            dst: Stream::newp(),
        }
    }

    fn process_one(&self, a: &T) -> T {
        *a + self.val
    }
}
map_block_macro_v2![AddConst<T>, std::ops::Add<Output = T>];
