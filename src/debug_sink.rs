//! Print values to stdout, for debugging.
use std::collections::HashMap;

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Streamp, Streamp2, TagPos};
use crate::Error;

/// Print values to stdout, for debugging.
pub struct DebugSink<T>
where
    T: Copy,
{
    src: Streamp2<T>,
}

#[allow(clippy::new_without_default)]
impl<T> DebugSink<T>
where
    T: Copy,
{
    /// Create new debug block.
    pub fn new(src: Streamp2<T>) -> Self {
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
        let i = self.src.read_buf().unwrap();
        i.iter().enumerate().for_each(|(_n, s)| {
            println!("debug: {:?}", s);
        });
        self.src.consume2(i.slice().len());

        // TODO: print tags.

        /*        let mut i = self.src.lock()?;
                let tags = i.tags().into_iter().map(|t| (t.pos(), t)).fold(
                    HashMap::new(),
                    |mut acc, (pos, tag)| {
                        acc.entry(pos).or_insert_with(Vec::new).push(tag);
                        acc
                    },
                );
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
                i.clear();
        */
        Ok(BlockRet::Noop)
    }
}
