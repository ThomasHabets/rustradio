//! Canary runs a lambda when it exits.
//!
//! It's an EOF detector.
use crate::Sample;
use crate::stream::{ReadStream, WriteStream};

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Canary<T: Sample, F>
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

impl<T: Sample, F> Drop for Canary<T, F>
where
    F: Fn() + Send,
{
    fn drop(&mut self) {
        (self.f)();
    }
}
