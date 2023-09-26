use crate::block::{Block, BlockRet, MapBlock};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{map_block_macro, Error};

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
}

impl<T> MapBlock<T> for MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
    Streamp<T>: From<StreamType>,
{
    fn process_one(&self, a: T) -> T {
        a * self.val
    }
}

map_block_macro![MultiplyConst, std::ops::Mul<Output = T>];
