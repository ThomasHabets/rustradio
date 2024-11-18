/*! Turn samples into text.

## Example

```
use rustradio::graph::Graph;
use rustradio::blocks::{ToText, ConstantSource, FileSink};
use rustradio::file_sink::Mode;
use rustradio::Float;
let src1 = ConstantSource::new(1.0);
let src2 = ConstantSource::new(-1.0);
let to_text = ToText::new(vec![src1.out(), src2.out()]);
let sink = FileSink::new(to_text.out(), "/dev/null".into(), Mode::Append)?;
let mut g = Graph::new();
g.add(Box::new(src1));
g.add(Box::new(src2));
g.add(Box::new(to_text));
g.add(Box::new(sink));
// g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
use crate::Error;

/// Turn samples into text.
///
/// Read from one or more streams, and produce a text file where each
/// line is one sample per stream, separated by spaces.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, out)]
pub struct ToText<T: Copy> {
    srcs: Vec<Streamp<T>>,
    #[rustradio(out)]
    dst: Streamp<u8>,
}

impl<T: Copy> ToText<T> {
    /// Create new ToText block.
    pub fn new(srcs: Vec<Streamp<T>>) -> Self {
        Self {
            srcs,
            dst: Stream::newp(),
        }
    }
}

impl<T: Copy + std::fmt::Debug> Block for ToText<T> {
    fn work(&mut self) -> Result<BlockRet, Error> {
        // TODO: This implementation locks and unlocks a lot, as it
        // aquires samples.  Ideally it should process
        // min(self.srcs...) samples, or until output buffer is full,
        // all in one lock.
        let mut empty = true;
        loop {
            let mut outs = Vec::new();
            for src in &mut self.srcs {
                let (i, tags) = src.read_buf()?;
                if i.is_empty() {
                    if empty {
                        return Ok(BlockRet::Noop);
                    } else {
                        return Ok(BlockRet::Ok);
                    }
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
            empty = false;
            let out = (outs.join(" ") + "\n").into_bytes();
            let mut o = self.dst.write_buf()?;
            if out.len() > o.len() {
                return Ok(BlockRet::Ok);
            }
            o.slice()[..out.len()].copy_from_slice(&out);
            o.produce(out.len(), &[]);
            for src in &mut self.srcs {
                let (i, _tags) = src.read_buf()?;
                i.consume(1);
            }
        }
    }
}
