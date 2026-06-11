//! Print values to stdout, for debugging.
use std::collections::HashMap;

use crate::{Result, Sample};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagPos};

/// Nocopy version of `DebugSink`.
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
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let Some((v, _tags)) = self.src.pop() else {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
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
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (i, tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        let l = i.len().min(self.dst.remaining());
        if l == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let wait_for_dst = l < i.len();

        let tags: HashMap<usize, Vec<Tag>> =
            tags.into_iter()
                .map(|t| (t.pos(), t))
                .fold(HashMap::new(), |mut acc, (pos, tag)| {
                    acc.entry(pos).or_default().push(tag);
                    acc
                });

        i.iter().take(l).enumerate().for_each(|(n, s)| {
            let ts = tags
                .get(&(n as TagPos))
                .map(|ts| {
                    ts.iter()
                        .map(|t| format!("{} => {:?}", t.key(), t.val()))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            self.dst.push(format!["{s:?} {ts}"], &[]);
        });
        i.consume(l);
        if wait_for_dst {
            Ok(BlockRet::WaitForStream(&self.dst, 1))
        } else {
            Ok(BlockRet::WaitForStream(&self.src, 1))
        }
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
    fn work(&mut self) -> Result<BlockRet<'_>> {
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
                .unwrap_or_default();
            println!("debug: {s:?} {ts}");
        });
        let l = i.slice().len();
        i.consume(l);
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockRet;
    use crate::stream::StreamWait;

    #[test]
    fn debug_filter_waits_for_output_when_output_fills() -> Result<()> {
        let src = ReadStream::from_slice(&vec![0u8; 1_001]);
        let (mut b, _out) = DebugFilter::new(src);
        let dst_id = b.dst.id();
        let ret = b.work()?;
        let BlockRet::WaitForStream(stream, 1) = ret else {
            panic!("unexpected return: {ret:?}");
        };
        assert_eq!(stream.id(), dst_id);
        Ok(())
    }
}
