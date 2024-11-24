//! Add a constant value to every sample.
use crate::stream::Streamp;

/// Add const value, implemented in terms of Map.
/// TODO: remove AddConst, below?
pub fn add_const<T>(src: Streamp<T>, val: T) -> crate::convert::Map<T, T, impl Fn(T) -> T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    crate::convert::MapBuilder::new(src, move |x| x + val)
        .name("add_const".into())
        .build()
}

/// AddConst adds a constant value to every sample.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out, sync)]
pub struct AddConst<T: Copy + std::ops::Add<Output = T>> {
    val: T,
    #[rustradio(in)]
    src: Streamp<T>,
    #[rustradio(out)]
    dst: Streamp<T>,
}

impl<T> AddConst<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    fn process_sync(&self, a: T) -> T {
        a + self.val
    }
}
