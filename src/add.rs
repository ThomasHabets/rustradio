//! Add two streams.
use crate::stream::{ReadStream, WriteStream};

/// Adds two streams, sample wise.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out, sync)]
pub struct Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    /// Hello world.
    #[rustradio(in)]
    a: ReadStream<T>,

    #[rustradio(in)]
    b: ReadStream<T>,

    #[rustradio(out)]
    dst: WriteStream<T>,
}
impl<T> Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    fn process_sync(&self, a: T, b: T) -> T {
        a + b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::blocks::VectorSource;
    use crate::Float;

    #[test]
    fn add_float() -> crate::Result<()> {
        // Testing VectorSource too, because why not.
        let input_a: Vec<_> = (0..10).map(|i| i as Float).collect();
        let (mut ablock, a) = VectorSource::new(input_a);
        ablock.work()?;

        let input_b: Vec<_> = (0..20).map(|i| 2.0 * (i as Float)).collect();
        let (mut bblock, b) = VectorSource::new(input_b);
        bblock.work()?;

        let (mut add, os) = Add::new(a, b);
        add.work()?;
        let (res, _) = os.read_buf()?;
        let want: Vec<_> = (0..10).map(|i| 3 * i).collect();
        let got: Vec<_> = res.slice().iter().map(|f| *f as usize).collect();
        assert_eq!(got, want);
        Ok(())
    }
}
