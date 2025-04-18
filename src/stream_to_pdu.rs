/*! Stream to PDU.

Turn a tagged stream to PDUs.

PDUs are marked in the stream as `true` when they start, and `false` when they end.

## Example

This example uses burst tagger to create the tags, and turn a stream
into burst PDUs.

Also see `examples/wpcr.rs`.

```
use rustradio::graph::{Graph, GraphRunner};
use rustradio::blocks::{FileSource, Tee, ComplexToMag2, SinglePoleIirFilter,BurstTagger,StreamToPdu};
use rustradio::Complex;
let (src, src_out) = FileSource::new("/dev/null")?;
let (tee, data, b) = Tee::new(src_out);
let (c2m, c2m_out) = ComplexToMag2::new(b);
let (iir, iir_out) = SinglePoleIirFilter::new(c2m_out, 0.01).unwrap();
let (burst, prev) = BurstTagger::new(data, iir_out, 0.0001, "burst");
let pdus = StreamToPdu::new(prev, "burst", 10_000, 50);
// pdus.out() now delivers bursts as Vec<Complex>
# Ok::<(), anyhow::Error>(())
```

 */
use std::collections::HashMap;

use log::{debug, trace};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagPos, TagValue};
use crate::{Result, Sample};

/// Stream to PDU block.
// TODO: implement proper EOF.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, nevereof)]
pub struct StreamToPdu<T> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<T>>,
    tag: String,
    buf: Vec<T>,
    endcounter: Option<usize>,
    max_size: usize,
    tail: usize,
}

impl<T> StreamToPdu<T> {
    /// Make new Stream to PDU block.
    pub fn new<S: Into<String>>(
        src: ReadStream<T>,
        tag: S,
        max_size: usize,
        tail: usize,
    ) -> (Self, NCReadStream<Vec<T>>) {
        let (dst, dr) = crate::stream::new_nocopy_stream();
        (
            Self {
                src,
                tag: tag.into(),
                dst,
                buf: Vec::with_capacity(max_size),
                endcounter: None,
                max_size,
                tail,
            },
            dr,
        )
    }
}

// If a given tag exists at the given position, return Some(that bool). Else
// return None.
fn get_tag_val_bool(tags: &HashMap<(TagPos, &str), &Tag>, pos: TagPos, key: &str) -> Option<bool> {
    if let Some(tag) = tags.get(&(pos, key)) {
        match tag.val() {
            TagValue::Bool(b) => Some(*b),
            _ => None,
        }
    } else {
        None
    }
}

impl<T> Block for StreamToPdu<T>
where
    T: Copy + Sample,
{
    fn work(&mut self) -> Result<BlockRet> {
        let (input, tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }

        // TODO: we actually only care about one single tag,
        // and I think we should drop the rest no matter what.
        let tags = tags
            .iter()
            .map(|t| ((t.pos(), t.key()), t))
            .collect::<HashMap<(TagPos, &str), &Tag>>();
        trace!("StreamToPdu: tags: {:?}", tags);

        for (i, sample) in input.iter().enumerate() {
            if let Some(0) = self.endcounter {
                let mut delme = Vec::with_capacity(self.max_size);
                std::mem::swap(&mut delme, &mut self.buf);
                debug!(
                    "StreamToPdu> got burst of size {} samples, {} bytes",
                    delme.len(),
                    delme.len() * T::size()
                );
                // TODO: record stream pos.
                self.dst.push(delme, &[]);
                self.endcounter = None;
            }
            if let Some(c) = self.endcounter {
                self.buf.push(*sample);
                self.endcounter = Some(c - 1);
            } else if let Some(tv) = get_tag_val_bool(&tags, i as TagPos, &self.tag) {
                if !tv {
                    // End of burst.
                    self.endcounter = Some(self.tail);
                } else {
                    // Start of burst, save first sample.
                    self.buf.push(*sample);
                }
            } else if !self.buf.is_empty() {
                // Burst continuation.
                self.buf.push(*sample);
            }
            if self.buf.len() > self.max_size {
                // Too long. Discard buffer and stop saving.
                self.buf.clear();
                self.endcounter = None;
            }
        }
        let n = input.len();
        input.consume(n);
        Ok(BlockRet::Again)
    }
}
