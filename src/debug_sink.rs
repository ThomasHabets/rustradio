//! Print values to stdout, for debugging.
use std::collections::HashMap;

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Streamp, Tag, TagPos};
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
        let mut i = self.src.lock()?;
        let tags = i
            .tags()
            .into_iter()
            .map(|t| (t.pos(), t))
            .collect::<HashMap<TagPos, Tag>>();
        i.iter().enumerate().for_each(|(n, s)| {
            println!("debug: {:?} {:?}", s, tags.get(&(n as TagPos)));
        });
        i.clear();
        Ok(BlockRet::Noop)
    }
}
