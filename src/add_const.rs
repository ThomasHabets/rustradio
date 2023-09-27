use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{map_block_macro_v2, Error};

pub struct AddConst<T> {
    val: T,
}

impl<T> AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    pub fn new(val: T) -> Self {
        Self { val }
    }
    fn process_one(&self, a: &T) -> T {
        *a + self.val
    }
}

map_block_macro_v2![AddConst<T>, std::ops::Add<Output = T>];
