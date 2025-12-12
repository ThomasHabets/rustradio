//! Canary runs a lambda when it exits.
//!
//! It's an EOF detector.
use crate::Sample;
use crate::stream::{ReadStream, WriteStream};

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync, bound = "T: Sample")]
pub struct Canary<T, F>
where
    F: Fn() + Send,
{
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
    f: F,
}

impl<T: Sample, F> Canary<T, F>
where
    F: Fn() + Send,
{
    fn process_sync(&mut self, s: T) -> T {
        s
    }
}

impl<T, F> Drop for Canary<T, F>
where
    F: Fn() + Send,
{
    fn drop(&mut self) {
        (self.f)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::block::{Block, BlockEOF, BlockRet};
    use crate::blocks::VectorSource;
    use std::sync::atomic::Ordering;

    #[test]
    fn canary() -> Result<()> {
        let c = std::sync::atomic::AtomicBool::new(false);
        let (mut ib, src) = VectorSource::new(vec![1u8, 2, 3]);
        ib.work()?;
        let (mut b, out) = Canary::new(src, || c.store(true, Ordering::Relaxed));
        assert!(!c.load(Ordering::Relaxed));
        let ret = b.work()?;
        assert!(matches![ret, BlockRet::WaitForStream(_, _)], "{ret:?}");
        drop(ib);
        drop(ret);
        assert!(b.eof());
        assert!(!c.load(Ordering::Relaxed));
        drop(b);
        assert!(c.load(Ordering::Relaxed));
        let (res, _) = out.read_buf()?;
        let got = res.slice().to_vec();
        let want = vec![1u8, 2, 3];
        assert_eq!(got, want);
        Ok(())
    }
}
