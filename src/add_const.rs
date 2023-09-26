use crate::block::{Block, BlockRet, MapBlock};
use crate::map_block_macro;
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

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
}

impl<T> MapBlock<T> for AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
    Streamp<T>: From<StreamType>,
{
    fn process_one(&self, a: T) -> T {
        a + self.val
    }
}

map_block_macro![AddConst];
