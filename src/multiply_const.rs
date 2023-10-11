//! Multiply stream by a constant value.
use std::sync::{Arc, Mutex};

use crate::map_block_macro_v2;
use crate::stream::Stream;

/// Multiply stream by a constant value.
pub struct MultiplyConst<T: Copy> {
    val: T,
    src: Arc<Mutex<Stream<T>>>,
    dst: Arc<Mutex<Stream<T>>>,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    /// Create new MultiplyConst block.
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
    fn process_one(&self, x: &T) -> T {
        *x * self.val
    }
}

map_block_macro_v2![MultiplyConst<T>, std::ops::Mul<Output = T>];
