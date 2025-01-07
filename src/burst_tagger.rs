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
use rustradio::graph::{Graph, GraphRunner};
use rustradio::blocks::{FileSource, Tee, ComplexToMag2, SinglePoleIIRFilter,BurstTagger,StreamToPdu};
use rustradio::Complex;
let (src, src_out) = FileSource::new("/dev/null", false)?;
let (tee, data, b) = Tee::new(src_out);
let (c2m, c2m_out) = ComplexToMag2::new(b);
let (iir, iir_out) = SinglePoleIIRFilter::new(c2m_out, 0.01).unwrap();
let (burst, burst_out) = BurstTagger::new(data, iir_out, 0.0001, "burst".to_string());
let pdus = StreamToPdu::new(burst_out, "burst".to_string(), 10_000, 50);
// pdus.out() now delivers bursts as Vec<Complex>
# Ok::<(), anyhow::Error>(())
```

## Constructor arguments

* `src`: Source data stream, will pass through and get tags.
* `trigger: Trigger stream.
* `threshold`: Threshold on trigger stream.
* `tag`: Tag name to add.

 */

use crate::stream::{ReadStream, Tag, TagValue, WriteStream};
use crate::Float;

/// Burst tagger:
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync_tag)]
pub struct BurstTagger<T: Copy> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(in)]
    trigger: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<T>,

    threshold: Float,
    tag: String,

    #[rustradio(default)]
    last: bool,
}

impl<T: Copy> BurstTagger<T> {
    fn process_sync_tags(&mut self, s: T, tv: Float, tags: &[Tag]) -> (T, Vec<Tag>) {
        let mut tags = tags.to_vec();
        let cur = tv > self.threshold;
        if cur != self.last {
            tags.push(Tag::new(
                0,
                self.tag.clone(),
                if cur {
                    TagValue::Bool(true)
                } else {
                    TagValue::Bool(false)
                },
            ));
        }
        self.last = cur;
        (s, tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::blocks::{VectorSink, VectorSource};
    use crate::Result;

    #[test]
    fn tag_it() -> Result<()> {
        let (mut src, src_out) = VectorSource::new((0..100).map(|i| i as u32).collect());
        let (mut trigger, trigger_out) = VectorSource::new(
            (0..100)
                .map(|i| match i as u32 {
                    0..80 => 0.1,
                    80..90 => 0.3,
                    90.. => 0.2,
                })
                .collect(),
        );
        let (mut b, b_out) = BurstTagger::new(src_out, trigger_out, 0.25, "burst".to_string());
        let mut sink = VectorSink::new(b_out, 1000);
        src.work()?;
        trigger.work()?;
        b.work()?;
        sink.work()?;
        let want: Vec<_> = (0..100).map(|i| i as u32).collect();
        assert_eq!(sink.data(), want);
        assert_eq!(
            sink.tags(),
            &[
                Tag::new(80, "burst".to_string(), TagValue::Bool(true)),
                Tag::new(90, "burst".to_string(), TagValue::Bool(false)),
            ]
        );
        Ok(())
    }
}
