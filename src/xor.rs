//! Xor two streams.
use crate::stream::{ReadStream, WriteStream};

/// Xors a constant value to every sample.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Xor<T>
where
    T: Copy + std::ops::BitXor<Output = T>,
{
    #[rustradio(in)]
    a: ReadStream<T>,
    #[rustradio(in)]
    b: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> Xor<T>
where
    T: Copy + std::ops::BitXor<Output = T>,
{
    fn process_sync(&self, a: T, b: T) -> T {
        a ^ b
    }
}
