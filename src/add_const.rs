//! Add a constant value to every sample.
use crate::stream::{ReadStream, WriteStream};

/// Add const value, implemented in terms of Map.
/// TODO: remove AddConst, below?
pub fn add_const<T>(
    src: ReadStream<T>,
    val: T,
) -> (crate::convert::Map<T, T, impl Fn(T) -> T>, ReadStream<T>)
where
    T: Copy + std::ops::Add<Output = T>,
{
    crate::convert::MapBuilder::new(src, move |x| x + val)
        .name("add_const".into())
        .build()
}

/// AddConst adds a constant value to every sample.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct AddConst<T: Copy + std::ops::Add<Output = T>> {
    val: T,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    fn process_sync(&self, a: T) -> T {
        a + self.val
    }
}
