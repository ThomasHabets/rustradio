//! Add a constant value to every sample.
use std::sync::{Arc, Mutex};

use crate::map_block_macro_v2;
use crate::stream::Stream;

/// AddConst adds a constant value to every sample.
pub struct AddConst<T>
where
    T: Copy,
{
    val: T,
    src: Arc<Mutex<Stream<T>>>,
    dst: Arc<Mutex<Stream<T>>>,
}

impl<T> AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Create a new AddConst, providing the constant to be added.
    pub fn new(src: Arc<Mutex<Stream<T>>>, val: T) -> Self {
        Self {
            val,
            src,
            dst: Arc::new(Mutex::new(Stream::<T>::new())),
        }
    }
    pub fn out(&self) -> Arc<Mutex<Stream<T>>> {
        self.dst.clone()
    }
    fn process_one(&self, a: &T) -> T {
        *a + self.val
    }
}
map_block_macro_v2![AddConst<T>, std::ops::Add<Output = T>];
