//! Print values to stdout, for debugging.
use std::collections::HashMap;

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Streamp, ReadStreamp, Tag, TagPos};
use crate::Error;

/// Print values to stdout, for debugging.
pub struct DebugSink<T>
where
    T: Copy,
{
    src: ReadStreamp<T>,
}

#[allow(clippy::new_without_default)]
impl<T> DebugSink<T>
where
    T: Copy,
{
    /// Create new debug block.
    pub fn new(src: ReadStreamp<T>) -> Self {
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
        let (i, tags) = self.src.read_buf()?;

        let tags: HashMap<usize, Vec<Tag>> =
            tags.into_iter()
                .map(|t| (t.pos(), t))
                .fold(HashMap::new(), |mut acc, (pos, tag)| {
                    acc.entry(pos).or_default().push(tag);
                    acc
                });

        i.iter().enumerate().for_each(|(n, s)| {
            let ts = tags
                .get(&(n as TagPos))
                .map(|ts| {
                    ts.iter()
                        .map(|t| format!("{} => {:?}", t.key(), t.val()))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or("".to_string());
            println!("debug: {:?} {}", s, ts);
        });
        let l = i.slice().len();
        i.consume(l);
        Ok(BlockRet::Noop)
    }
}
