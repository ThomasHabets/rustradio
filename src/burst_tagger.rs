/*! Burst tagger.

This block takes two inputs. One data stream, of any type, that will
be passed through as-is. And a threshold stream, of type Float, that
when it goes above the threshold, adds a tag to the data stream with
the value `true`. When it goes below, it adds the same tag with the
value `false`.

The float input should likely be filtered with an IIR filter.

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

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp, Tag, TagValue};
use crate::{Error, Float};

/// Burst tagger:
pub struct BurstTagger<T> {
    src: Streamp<T>,
    threshold: Float,
    trigger: Streamp<Float>,
    dst: Streamp<T>,
    tag: String,
    last: bool,
}

impl<T> BurstTagger<T> {
    /// Create new burst tagger.
    ///
    /// * src: Source data stream, will pass through and get tags.
    /// * trigger: Trigger stream.
    /// * threshold: Threshold on trigger stream.
    /// * tag: Tag name to add.
    pub fn new(src: Streamp<T>, trigger: Streamp<Float>, threshold: Float, tag: String) -> Self {
        Self {
            src,
            trigger,
            threshold,
            tag,
            dst: new_streamp(),
            last: false,
        }
    }

    /// Get output stream.
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T> Block for BurstTagger<T>
where
    T: Copy,
{
    fn block_name(&self) -> &'static str {
        "Burst Tagger"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let (input, mut tags) = self.src.read_buf()?;
        let (trigger, _) = self.trigger.read_buf()?;
        let mut o = self.dst.write_buf()?;
        let n = std::cmp::min(input.len(), trigger.len());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
        let n = std::cmp::min(n, o.len());
        if n == 0 {
            return Ok(BlockRet::Ok);
        }

        let mut v = Vec::with_capacity(input.len());
        for (i, (s, tv)) in input.iter().zip(trigger.iter()).enumerate().take(n) {
            let cur = *tv > self.threshold;
            if cur != self.last {
                tags.push(Tag::new(
                    i,
                    self.tag.clone(),
                    if cur {
                        TagValue::Bool(true)
                    } else {
                        TagValue::Bool(false)
                    },
                ));
            }
            self.last = cur;
            v.push(*s);
        }
        o.slice()[..n].clone_from_slice(&v);
        o.produce(n, &tags);
        input.consume(n);
        trigger.consume(n);
        Ok(BlockRet::Ok)
    }
}
