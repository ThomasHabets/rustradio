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
    fn simple() -> Result<()> {
        // Set up input.
        let samps: Vec<_> = (0..10).map(|i| i as Float).collect();
        let (mut iblock, i) = VectorSource::new(samps.clone());
        iblock.work()?;

        // Run tee.
        let (mut tee, out1, out2) = Tee::new(i);
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
