use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{map_block_macro_v2, Error};

pub struct MultiplyConst<T> {
    val: T,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    pub fn new(val: T) -> Self {
        Self { val }
    }
    fn process_one(&self, x: &T) -> T {
        *x * self.val
    }
}

map_block_macro_v2![MultiplyConst<T>, std::ops::Mul<Output = T>];
