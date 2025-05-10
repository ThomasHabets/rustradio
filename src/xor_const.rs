//! Xor a constant value with every sample.
use crate::stream::{ReadStream, WriteStream};

/// XorConst xors a constant value to every sample.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct XorConst<T>
where
    T: Copy + std::ops::BitXor<Output = T> + Send + Sync + 'static,
{
    val: T,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> XorConst<T>
where
    T: Copy + std::ops::BitXor<Output = T> + Send + Sync + 'static,
{
    fn process_sync(&mut self, a: T) -> T {
        a ^ self.val
    }
}
