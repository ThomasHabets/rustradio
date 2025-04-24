//! Tee a stream.

use crate::stream::{ReadStream, WriteStream};

/// Tee a stream into two.
///
/// Every input sample is copied into each output stream.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Tee<T: Copy> {
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst1: WriteStream<T>,
    #[rustradio(out)]
    dst2: WriteStream<T>,
}
impl<T: Copy> Tee<T> {
    fn process_sync(&self, s: T) -> (T, T) {
        (s, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::block::{Block, BlockRet};
    use crate::blocks::{VectorSink, VectorSource};
    use crate::{Float, Result};

    #[test]
    fn simple() -> Result<()> {
        // Set up input.
        let samps: Vec<_> = (0..10).map(|i| i as Float).collect();
        let (mut iblock, i) = VectorSource::new(samps.clone());
        iblock.work()?;

        // Run tee.
        let (mut tee, out1, out2) = Tee::new(i);
        let ret = tee.work()?;
        assert!(matches![ret, BlockRet::Again], "{:?}", ret);
        drop(ret);
        let ret = tee.work()?;
        assert!(matches![ret, BlockRet::WaitForStream(_, 1)], "{:?}", ret);

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
