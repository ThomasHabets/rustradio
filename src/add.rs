//! Add two streams.
use crate::stream::{Stream, Streamp};

/// Adds two streams, sample wise.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out, sync)]
pub struct Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Hello world.
    #[rustradio(in)]
    a: Streamp<T>,

    #[rustradio(in)]
    b: Streamp<T>,

    #[rustradio(out)]
    dst: Streamp<T>,
}

impl<T> Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    fn process_sync(&self, a: T, b: T) -> T {
        a + b
    }
}
