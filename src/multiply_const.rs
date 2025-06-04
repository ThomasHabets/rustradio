//! Multiply stream by a constant value.
use crate::Sample;
use crate::stream::{ReadStream, WriteStream};

/// Multiply stream by a constant value.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync, bound = "T: Sample + std::ops::Mul<Output=T>")]
pub struct MultiplyConst<T> {
    val: T,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> MultiplyConst<T>
where
    T: Sample + std::ops::Mul<Output = T>,
{
    fn process_sync(&self, x: T) -> T {
        x * self.val
    }
}
