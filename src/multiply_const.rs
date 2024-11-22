//! Multiply stream by a constant value.
use crate::stream::Streamp;

/// Multiply stream by a constant value.
///
/// TODO: replace with a mapper?
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out, sync)]
pub struct MultiplyConst<T: Copy + std::ops::Mul<Output = T>> {
    val: T,
    #[rustradio(in)]
    src: Streamp<T>,
    #[rustradio(out)]
    dst: Streamp<T>,
}

impl<T> MultiplyConst<T>
where
    T: Copy + std::ops::Mul<Output = T>,
{
    fn process_sync(&self, x: T) -> T {
        x * self.val
    }
}
