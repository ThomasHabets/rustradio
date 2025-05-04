//! Burst tagger.
//!
//! Add tags to a stream to indicate stand and end of a burst. Does not
//! otherwise modify the stream.

use std::borrow::Cow;

use crate::Float;
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};

/// Burst tagger
///
/// This block takes two inputs. One data stream, of any type, that will
/// be passed through as-is. And a threshold stream, of type Float, that
/// when it goes above the threshold, adds a tag to the data stream with
/// the value `true`. When it goes below, it adds the same tag with the
/// value `false`.
///
/// The float input should likely be filtered with an IIR filter.
///
/// ## Example
///
/// This example uses burst tagger to create the tags, and turn a stream
/// into burst PDUs.
///
/// Also see `examples/wpcr.rs`.
///
/// ```
/// use rustradio::graph::{Graph, GraphRunner};
/// use rustradio::blocks::{FileSource, Tee, ComplexToMag2, SinglePoleIirFilter,BurstTagger,StreamToPdu};
/// use rustradio::Complex;
/// let (src, src_out) = FileSource::new("/dev/null")?;
/// let (tee, data, b) = Tee::new(src_out);
/// let (c2m, c2m_out) = ComplexToMag2::new(b);
/// let (iir, iir_out) = SinglePoleIirFilter::new(c2m_out, 0.01).unwrap();
/// let (burst, burst_out) = BurstTagger::new(data, iir_out, 0.0001, "burst");
/// let pdus = StreamToPdu::new(burst_out, "burst", 10_000, 50);
/// // pdus.out() now delivers bursts as Vec<Complex>
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// ## Constructor arguments
///
/// * `src`: Source data stream, will pass through and get tags.
/// * `trigger: Trigger stream.
/// * `threshold`: Threshold on trigger stream.
/// * `tag`: Tag name to add.
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

    #[rustradio(into)]
    tag: String,

    #[rustradio(default)]
    last: bool,
}

impl<T: Copy> BurstTagger<T> {
    fn process_sync_tags<'a>(
        &mut self,
        s: T,
        tags: &'a [Tag],
        tv: Float,
        _tv_tags: &[Tag],
    ) -> (T, Cow<'a, [Tag]>) {
        let cur = tv > self.threshold;
        let tags = if cur != self.last {
            let mut owned_tags: Vec<Tag> = tags.to_vec();
            owned_tags.push(Tag::new(0, self.tag.clone(), TagValue::Bool(cur)));
            Cow::Owned(owned_tags)
        } else {
            Cow::Borrowed(tags)
        };
        self.last = cur;
        (s, tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::block::Block;
    use crate::blocks::{VectorSink, VectorSource};

    fn tag_compare(left: &[Tag], right: &[Tag]) {
        let mut left = left.to_vec();
        left.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut right = right.to_vec();
        right.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(left, right);
    }

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
        let (mut b, b_out) = BurstTagger::new(src_out, trigger_out, 0.25, "burst");
        let mut sink = VectorSink::new(b_out, 1000);
        src.work()?;
        trigger.work()?;
        b.work()?;
        sink.work()?;
        let want: Vec<_> = (0..100).map(|i| i as u32).collect();
        assert_eq!(sink.hook().data().samples(), want);
        tag_compare(
            sink.hook().data().tags(),
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(90, "burst", TagValue::Bool(false)),
                Tag::new(80, "burst", TagValue::Bool(true)),
            ],
        );
        Ok(())
    }
}
