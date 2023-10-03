//! Print values to stdout, for debugging.
use anyhow::Result;

use crate::block::{get_input, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::Error;

/// Print values to stdout, for debugging.
pub struct DebugSink<T> {
    dummy: std::marker::PhantomData<T>,
}

#[allow(clippy::new_without_default)]
impl<T> DebugSink<T> {
    /// Create new debug block.
    pub fn new() -> Self {
        Self {
            dummy: std::marker::PhantomData,
        }
    }
}

impl<T> Block for DebugSink<T>
where
    T: Copy + std::fmt::Debug + Default,
    Streamp<T>: From<StreamType>,
{
    fn block_name(&self) -> &'static str {
        "DebugSink"
    }
    fn work(&mut self, r: &mut InputStreams, _w: &mut OutputStreams) -> Result<BlockRet, Error> {
        get_input(r, 0).borrow().iter().for_each(|s: &T| {
            println!("debug: {:?}", s);
        });
        get_input(r, 0).borrow_mut().clear();
        Ok(BlockRet::Ok)
    }
}
