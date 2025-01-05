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
        let input_a: Vec<_> = (0..10).map(|i| i as Float).collect();
        let mut a = VectorSource::new(input_a);
        a.work()?;

        let input_b: Vec<_> = (0..20).map(|i| 2.0 * (i as Float)).collect();
        let mut b = VectorSource::new(input_b);
        b.work()?;

        let mut add = Add::new(a.out(), b.out());
        add.work()?;
        let os = add.out();
        let (res, _) = os.read_buf()?;
        let want: Vec<_> = (0..10).map(|i| 3 * i).collect();
        let got: Vec<_> = res.slice().iter().map(|f| *f as usize).collect();
        assert_eq!(got, want);
        Ok(())
    }
}
