//! Tee a stream

use anyhow::Result;

use crate::stream::{ReadStream, WriteStream};

/// Tee
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
