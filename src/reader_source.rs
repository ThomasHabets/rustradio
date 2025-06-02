use std::io::Read;

use log::warn;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, Sample};

/// Arbitrary reader source.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct ReaderSource<T: Sample> {
    reader: Box<dyn Read + Send>,
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample> ReaderSource<T> {
    pub fn new<R: Read + Send + 'static>(reader: R) -> (Self, ReadStream<T>) {
        let (dst, r) = crate::stream::new_stream();
        (
            Self {
                reader: Box::new(reader),
                buf: Vec::new(),
                dst,
            },
            r,
        )
    }
}

impl<T> Block for ReaderSource<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
        let mut o = self.dst.write_buf()?;
        let size = T::size();
        let mut buffer = vec![0; o.len()];
        // TODO: this read blocks.
        let n = self.reader.read(&mut buffer[..])?;
        if n == 0 {
            warn!("TCP connection closed?");
            return Ok(BlockRet::EOF);
        }
        let mut v = Vec::with_capacity(n / size + 1);

        let mut steal = 0;
        if !self.buf.is_empty() {
            steal = size - self.buf.len();
            self.buf.extend(&buffer[0..steal]);
            v.push(T::parse(&self.buf)?);
            self.buf.clear();
        }
        let remaining = (n - steal) % size;
        for pos in (steal..(n - remaining)).step_by(size) {
            v.push(T::parse(&buffer[pos..pos + size])?);
        }
        self.buf.extend(&buffer[n - remaining..n]);
        let n = v.len();
        o.fill_from_iter(v);
        o.produce(n, &[]);
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_source() -> Result<()> {
        let data = b"hello world";
        let r = std::io::Cursor::new(data);
        let (mut b, prev) = ReaderSource::<u8>::new(r);
        b.work()?;
        let (o, _) = prev.read_buf()?;
        assert_eq!(o.slice(), b"hello world");
        Ok(())
    }
}
