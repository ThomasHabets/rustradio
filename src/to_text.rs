/*! Turn samples into text.

## Example

```
use rustradio::graph::{Graph, GraphRunner, GraphRunnerExt};
use rustradio::blocks::{ToText, ConstantSource, FileSink};
use rustradio::file_sink::Mode;
use rustradio::Float;
let (src1, src1_out) = ConstantSource::new(1.0)?;
let (src2, src2_out) = ConstantSource::new(-1.0)?;
let (to_text, to_text_out) = ToText::new(vec![src1_out, src2_out])?;
let sink = FileSink::new(to_text_out, "/dev/null", Mode::Append)?;
let mut g = Graph::new();
g.add(src1);
g.add(src2);
g.add(to_text);
g.add(sink);
// g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
use std::fmt::Write;

use log::debug;

use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, Sample};

/// Turn samples into text.
///
/// Read from one or more streams, and produce a text file where each
/// line is one sample per stream, separated by spaces.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof, bound = "T: Sample")]
pub struct ToText<T> {
    srcs: Vec<ReadStream<T>>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
}

impl<T: Sample> ToText<T> {
    /// Create new `ToText` block.
    pub fn new(srcs: Vec<ReadStream<T>>) -> Result<(Self, ReadStream<u8>)> {
        if srcs.is_empty() {
            debug!("ToText set up with zero inputs. This is probably not intentional");
        }
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((Self { srcs, dst }.with_output_metadata(), dr))
    }
}

impl<T> BlockEOF for ToText<T> {
    fn eof(&mut self) -> bool {
        self.srcs.iter().any(super::stream::ReadStream::eof)
    }
}

impl<T: Sample + std::fmt::Debug> Block for ToText<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        if self.srcs.is_empty() {
            return Ok(BlockRet::EOF);
        }
        // TODO: This implementation locks and unlocks a lot, as it
        // aquires samples.  Ideally it should process
        // min(self.srcs...) samples, or until output buffer is full,
        // all in one lock.
        let cur_block = 'outer: loop {
            let mut outs = Vec::new();
            for (cur_block, src) in self.srcs.iter_mut().enumerate() {
                let i = src.read_buf()?;
                if i.is_empty() {
                    break 'outer cur_block;
                }
                let mut s: String = format!("{:?}", i.slice()[0]);
                let mut tags = i.tags().take_while(|tag| tag.pos() == 0).peekable();
                if tags.peek().is_some() {
                    s += " (";
                    for tag in tags {
                        let _ = write!(s, "{}={:?}", tag.key(), tag.val());
                    }
                    s += ")";
                }
                outs.push(s);
            }
            let out = (outs.join(" ") + "\n").into_bytes();
            let mut o = self.dst.write_buf()?;
            let capacity = self.dst.capacity();
            if out.len() > capacity {
                return Err(crate::Error::msg(format!(
                    "ToText row of {} bytes exceeds buffer capacity of {capacity} bytes",
                    out.len()
                )));
            }
            if out.len() > o.len() {
                return Ok(BlockRet::WaitForStream(&self.dst, out.len()));
            }
            o.slice()[..out.len()].copy_from_slice(&out);
            o.produce(out.len(), &[]);
            for src in &mut self.srcs {
                let i = src.read_buf()?;
                i.consume_discard_tags(1);
            }
        };
        Ok(BlockRet::WaitForStream(&self.srcs[cur_block], 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    #[test]
    fn eof_when_one_input_closes() -> crate::Result<()> {
        let (mut block, _output) = ToText::new(vec![
            ReadStream::from_slice(&[]),
            ReadStream::from_slice(&[1u8]),
        ])?;
        assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(BlockEOF::eof(&mut block));
        Ok(())
    }
}
