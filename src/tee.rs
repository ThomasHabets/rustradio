//! Tee a stream.
use crate::Sample;
use crate::stream::{ReadStream, WriteStream};

/// Tee a stream into two.
///
/// Every input sample is copied into each output stream. Tags are written to
/// both streams.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Tee<T: Sample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst1: WriteStream<T>,
    #[rustradio(out)]
    dst2: WriteStream<T>,
}

impl<T: Sample> Tee<T> {
    fn process_sync(&self, s: T) -> (T, T) {
        (s, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Cow;

    use crate::block::{Block, BlockRet};
    use crate::blocks::{VectorSink, VectorSource};
    use crate::stream::Tag;
    use crate::{Float, Result};

    /// A version of Tee that uses process_sync_tags.
    ///
    /// It's just here for unit tests to assure multiple return values works as
    /// expected.
    #[derive(rustradio_macros::Block)]
    #[rustradio(crate, new, sync_tag)]
    struct Tee<T: Sample> {
        #[rustradio(in)]
        src: ReadStream<T>,
        #[rustradio(out)]
        dst1: WriteStream<T>,
        #[rustradio(out)]
        dst2: WriteStream<T>,
    }

    impl<T: Sample> Tee<T> {
        fn process_sync_tags<'a>(
            &self,
            s: T,
            ts: &'a [Tag],
        ) -> (T, Cow<'a, [Tag]>, T, Cow<'a, [Tag]>) {
            (s, Cow::Borrowed(ts), s, Cow::Owned(vec![]))
        }
    }

    #[test]
    fn plain_tee_respects_short_output_windows_and_preserves_tags() -> Result<()> {
        use crate::stream::{TagValue, new_stream};

        let (writer, input) = new_stream()?;
        let (mut tee, first, second) = super::Tee::new(input)?;
        let mut padding = tee.dst2.write_buf()?;
        let padding_len = padding.len() - 2;
        padding.slice()[..padding_len].fill(0u32);
        padding.produce(padding_len, &[]);
        let samples = [11, 22, 33, 44, 55];
        let tags = [
            Tag::new(1, "early", TagValue::Bool(true)),
            Tag::new(2, "boundary", TagValue::Bool(true)),
            Tag::new(4, "late", TagValue::Bool(true)),
        ];
        let mut input = writer.write_buf()?;
        input.fill_from_slice(&samples);
        input.produce(samples.len(), &tags);
        drop(writer);

        assert!(matches!(tee.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(matches!(tee.work()?, BlockRet::WaitForStream(_, 1)));
        second.read_buf()?.0.consume(padding_len);
        for output in [&first, &second] {
            let (window, output_tags) = output.read_buf()?;
            assert_eq!(window.slice(), &samples[..2]);
            assert_eq!(output_tags, &tags[..1]);
            window.consume(2);
        }
        tee.work()?;
        for output in [&first, &second] {
            let (window, output_tags) = output.read_buf()?;
            assert_eq!(window.slice(), &samples[2..]);
            assert_eq!(
                output_tags,
                [
                    Tag::new(0, "boundary", TagValue::Bool(true)),
                    Tag::new(2, "late", TagValue::Bool(true)),
                ]
            );
            window.consume(3);
        }
        assert!(tee.src.eof());
        Ok(())
    }

    #[test]
    fn simple() -> Result<()> {
        // Set up input.
        let samps: Vec<_> = (0..10).map(|i| i as Float).collect();
        let (mut iblock, i) = VectorSource::new(samps.clone())?;
        iblock.work()?;

        // Run tee.
        let (mut tee, out1, out2) = Tee::new(i)?;
        let ret = tee.work()?;
        assert!(matches![ret, BlockRet::WaitForStream(_, 1)], "{ret:?}");

        // Check left side.
        {
            let mut o = VectorSink::new(out1, 100);
            o.work()?;
            assert_eq!(o.hook().data().samples(), samps);
        }

        // Check right side.
        {
            let mut o = VectorSink::new(out2, 100);
            o.work()?;
            assert_eq!(o.hook().data().samples(), samps);
        }
        Ok(())
    }
}
