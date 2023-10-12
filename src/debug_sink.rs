//! Print values to stdout, for debugging.

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Streamp;
use crate::Error;

/// Print values to stdout, for debugging.
pub struct DebugSink<T>
where
    T: Copy,
{
    src: Streamp<T>,
}

#[allow(clippy::new_without_default)]
impl<T> DebugSink<T>
where
    T: Copy,
{
    /// Create new debug block.
    pub fn new(src: Streamp<T>) -> Self {
        Self { src }
    }
}

impl<T> Block for DebugSink<T>
where
    T: Copy + std::fmt::Debug + Default,
{
    fn block_name(&self) -> &'static str {
        "DebugSink"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut i = self.src.lock().unwrap();
        i.iter().for_each(|s: &T| {
            println!("debug: {:?}", s);
        });
        i.clear();
        Ok(BlockRet::Noop)
    }
}
