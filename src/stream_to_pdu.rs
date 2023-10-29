/*! Stream to PDU.

Turn a tagged stream to PDUs.

PDUs are marked in the stream as `true` when they start, and `false` when they end.

## Example

This example uses burst tagger to create the tags, and turn a stream
into burst PDUs.

Also see `examples/wpcr.rs`.

```
use rustradio::graph::Graph;
use rustradio::blocks::{FileSource, Tee, ComplexToMag2, SinglePoleIIRFilter,BurstTagger,StreamToPdu};
use rustradio::Complex;
let src = FileSource::new("/dev/null", false)?;
let tee = Tee::new(src.out());
let (data,b) = tee.out();
let c2m = ComplexToMag2::new(b);
let iir = SinglePoleIIRFilter::new(c2m.out(), 0.01).unwrap();
let burst = BurstTagger::new(data, c2m.out(), 0.0001, "burst".to_string());
let pdus = StreamToPdu::new(burst.out(), "burst".to_string(), 10_000, 50);
// pdus.out() now delivers bursts as Vec<Complex>
# Ok::<(), anyhow::Error>(())
```

 */
use std::collections::HashMap;

use log::{info, trace};

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp, Tag, TagPos, TagValue};
use crate::{Error, Sample};

/// Stream to PDU block.
pub struct StreamToPdu<T> {
    src: Streamp<T>,
    dst: Streamp<Vec<T>>,
    tag: String,
    buf: Vec<T>,
    endcounter: Option<usize>,
    max_size: usize,
    tail: usize,
}

impl<T> StreamToPdu<T> {
    /// Make new Stream to PDU block.
    pub fn new(src: Streamp<T>, tag: String, max_size: usize, tail: usize) -> Self {
        Self {
            src,
            tag,
            dst: new_streamp(),
            buf: Vec::with_capacity(max_size),
            endcounter: None,
            max_size,
            tail,
        }
    }
    /// Get output PDU stream.
    pub fn out(&self) -> Streamp<Vec<T>> {
        self.dst.clone()
    }
}

fn get_tag_val_bool(tags: &HashMap<(TagPos, String), Tag>, pos: TagPos, key: &str) -> Option<bool> {
    if let Some(tag) = tags.get(&(pos, key.to_string())) {
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
    fn block_name(&self) -> &'static str {
        "StreamToPdu"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        if input.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        // TODO: we actually only care about one single tag,
        // and I think we should drop the rest no matter what.
        let tags = input
            .tags()
            .into_iter()
            .map(|t| ((t.pos(), t.key().to_string()), t))
            .collect::<HashMap<(TagPos, String), Tag>>();
        trace!("StreamToPdu: tags: {:?}", tags);
        for (i, sample) in input.iter().enumerate() {
            if let Some(0) = self.endcounter {
                let mut delme = Vec::with_capacity(self.max_size);
                std::mem::swap(&mut delme, &mut self.buf);
                info!(
                    "StreamToPdu> got burst of size {} samples, {} bytes",
                    delme.len(),
                    delme.len() * T::size()
                );
                self.dst.lock()?.push(delme);
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
        input.clear();
        Ok(BlockRet::Ok)
    }
}
