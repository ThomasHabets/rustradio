/*! Turn samples into text.

## Example

```
use rustradio::graph::{Graph, GraphRunner};
use rustradio::blocks::{ToText, ConstantSource, FileSink};
use rustradio::file_sink::Mode;
use rustradio::Float;
let (src1, src1_out) = ConstantSource::new(1.0);
let (src2, src2_out) = ConstantSource::new(-1.0);
let (to_text, to_text_out) = ToText::new(vec![src1_out, src2_out]);
let sink = FileSink::new(to_text_out, "/dev/null", Mode::Append)?;
let mut g = Graph::new();
g.add(Box::new(src1));
g.add(Box::new(src2));
g.add(Box::new(to_text));
g.add(Box::new(sink));
// g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
use crate::{Result, Sample};

use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::{ReadStream, WriteStream};

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

impl<T> ToText<T> {
    /// Create new ToText block.
    #[must_use]
    pub fn new(srcs: Vec<ReadStream<T>>) -> (Self, ReadStream<u8>) {
        let (dst, dr) = crate::stream::new_stream();
        (Self { srcs, dst }, dr)
    }
}

impl<T> BlockEOF for ToText<T> {
    fn eof(&mut self) -> bool {
        self.srcs.iter().all(|s| s.eof())
    }
}

impl<T: Sample + std::fmt::Debug> Block for ToText<T> {
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: This implementation locks and unlocks a lot, as it
        // aquires samples.  Ideally it should process
        // min(self.srcs...) samples, or until output buffer is full,
        // all in one lock.
        let cur_block = 'outer: loop {
            let mut outs = Vec::new();
            for (cur_block, src) in self.srcs.iter_mut().enumerate() {
                let (i, tags) = src.read_buf()?;
                if i.is_empty() {
                    break 'outer cur_block;
                }
                let mut s: String = format!("{:?}", i.slice()[0]);
                if !tags.is_empty() && tags[0].pos() == 0 {
                    s += " (";
                    for tag in tags {
                        if tag.pos() != 0 {
                            break;
                        }
                        s += &format!("{}={:?}", tag.key(), tag.val());
                    }
                    s += ")";
                }
                outs.push(s);
            }
            let out = (outs.join(" ") + "\n").into_bytes();
            let mut o = self.dst.write_buf()?;
            if out.len() > o.len() {
                return Ok(BlockRet::WaitForStream(&self.dst, out.len()));
            }
            o.slice()[..out.len()].copy_from_slice(&out);
            o.produce(out.len(), &[]);
            for src in &mut self.srcs {
                let (i, _tags) = src.read_buf()?;
                i.consume(1);
            }
        };
        Ok(BlockRet::WaitForStream(&self.srcs[cur_block], 1))
    }
}
