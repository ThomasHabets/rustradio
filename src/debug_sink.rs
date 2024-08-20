//! Print values to stdout, for debugging.
use std::collections::HashMap;

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{NoCopyStream, NoCopyStreamp, Streamp, Tag, TagPos};
use crate::Error;

/// Nocopy version of DebugSink.
// TODO: maybe merge with DebugSink using an enum?
pub struct DebugSinkNoCopy<T> {
    src: NoCopyStreamp<T>,
}

impl<T> DebugSinkNoCopy<T> {
    /// Create new debug block.
    pub fn new(src: NoCopyStreamp<T>) -> Self {
        Self { src }
    }
}

impl<T> Block for DebugSinkNoCopy<T>
where
    T: std::fmt::Debug + Default,
{
    fn block_name(&self) -> &str {
        "DebugSinkNoCopy"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (v, _tags) = match self.src.pop() {
            None => return Ok(BlockRet::Noop),
            Some(x) => x,
        };

        // TODO: print tags.
        /*
        let tags: HashMap<usize, Vec<Tag>> =
                    tags.into_iter()
                        .map(|t| (t.pos(), t))
                        .fold(HashMap::new(), |mut acc, (pos, tag)| {
                            acc.entry(pos).or_default().push(tag);
                            acc
                        });
                 */

        println!("debug: {:?}", v);
        Ok(BlockRet::Ok)
    }
}

/// Debug filter turning samples into strings.
pub struct DebugFilter<T>
where
    T: Copy,
{
    src: Streamp<T>,
    dst: NoCopyStreamp<String>,
}

impl<T> DebugFilter<T>
where
    T: Copy,
{
    /// Create new debug block.
    pub fn new(src: Streamp<T>) -> Self {
        Self {
            src,
            dst: NoCopyStream::newp(),
        }
    }
    /// Return the output stream.
    pub fn out(&self) -> NoCopyStreamp<String> {
        self.dst.clone()
    }
}

impl<T> Block for DebugFilter<T>
where
    T: Copy + std::fmt::Debug,
{
    fn block_name(&self) -> &str {
        "DebugFilter"
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
            self.dst.push(format!["{:?} {}", s, ts], &[]);
        });
        let l = i.slice().len();
        i.consume(l);
        Ok(BlockRet::Noop)
    }
}

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
    fn block_name(&self) -> &str {
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
