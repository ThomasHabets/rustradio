//! Xor a constant value with every sample.
use crate::stream::Streamp;

/// XorConst xors a constant value to every sample.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out, sync)]
pub struct XorConst<T>
where
    T: Copy + std::ops::BitXor<Output = T>,
{
    val: T,
    #[rustradio(in)]
    src: Streamp<T>,
    #[rustradio(out)]
    dst: Streamp<T>,
}

impl<T> XorConst<T>
where
    T: Copy + std::ops::BitXor<Output = T>,
{
    fn process_sync(&mut self, a: T) -> T {
        a ^ self.val
    }
}
