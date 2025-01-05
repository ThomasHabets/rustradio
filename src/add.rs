//! Add two streams.
use crate::stream::{ReadStream, WriteStream};

/// Adds two streams, sample wise.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out)]
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
    fn process_sync_tags(
        &mut self,
        a: T,
        b: T,
        tags: &[crate::stream::Tag],
    ) -> (T, Vec<crate::stream::Tag>) {
        (self.process_sync(a, b), tags.to_vec())
    }
}
impl<T> crate::block::Block for Add<T>
where
    T: Copy + std::ops::Add<Output = T>,
{
    fn work(&mut self) -> Result<crate::block::BlockRet, crate::Error> {
        let (a, _) = self.a.read_buf()?;
        let (b, _) = self.b.read_buf()?;
        let n = [a.len(), b.len()]
            .iter()
            .fold(usize::MAX, |min, &x| min.min(x));
        if n == 0 {
            return Ok(crate::block::BlockRet::Noop);
        }
        let mut dst = self.dst.write_buf()?;
        let n = [dst.len()].iter().fold(n, |min, &x| min.min(x));
        let mut otags = Vec::new();
        let it = a
            .iter()
            .take(n)
            .zip(b.iter())
            .enumerate()
            .map(|(pos, (a, b))| {
                let (s, ts) = self.process_sync_tags(*a, *b, &[]);
                for tag in ts {
                    otags.push(crate::stream::Tag::new(
                        pos,
                        tag.key().into(),
                        tag.val().clone(),
                    ));
                }
                s
            });
        for (samp, w) in it.zip(dst.slice().iter_mut()) {
            *w = samp;
        }
        a.consume(n);
        b.consume(n);
        dst.produce(n, &otags);
        Ok(crate::block::BlockRet::Ok)
    }
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
