//! Print values to stdout, for debugging.
use std::collections::HashMap;

use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagPos};

/// Nocopy version of DebugSink.
// TODO: maybe merge with DebugSink using an enum?
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct DebugSinkNoCopy<T: Send + Sync + 'static> {
    #[rustradio(in)]
    src: NCReadStream<T>,
}

impl<T> Block for DebugSinkNoCopy<T>
where
    T: std::fmt::Debug + Default + Send + Sync + 'static,
{
    fn work(&mut self) -> Result<BlockRet> {
        let (v, _tags) = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
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

        println!("debug: {v:?}");
        Ok(BlockRet::Again)
    }
}

/// Debug filter turning samples into strings.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct DebugFilter<T>
where
    T: Sample,
{
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: NCWriteStream<String>,
}

impl<T> Block for DebugFilter<T>
where
    T: Sample + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
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
            self.dst.push(format!["{s:?} {ts}"], &[]);
        });
        let l = i.slice().len();
        i.consume(l);
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}

/// Print values to stdout, for debugging.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct DebugSink<T>
where
    T: Sample,
{
    #[rustradio(in)]
    src: ReadStream<T>,
}

//#[allow(clippy::new_without_default)]

impl<T> Block for DebugSink<T>
where
    T: Sample + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
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
            println!("debug: {s:?} {ts}");
        });
        let l = i.slice().len();
        i.consume(l);
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}
