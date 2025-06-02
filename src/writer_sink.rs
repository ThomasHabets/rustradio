use std::io::Write;

use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;
use crate::{Result, Sample};

/// Arbitrary writer sink.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct WriterSink<T: Sample> {
    writer: Box<dyn Write + Send>,
    #[rustradio(in)]
    src: ReadStream<T>,
}

impl<T: Sample> WriterSink<T> {
    pub fn new<R: Write + Send + 'static>(src: ReadStream<T>, writer: R) -> Self {
        Self {
            writer: Box::new(writer),
            src,
        }
    }
}

impl<T> Block for WriterSink<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: make nonblock.
        loop {
            let (i, _) = self.src.read_buf()?;
            if i.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            // TODO: very inefficient.
            let b = i.slice()[0].serialize();
            let rc = self.writer.write(&b)?;
            assert_eq!(rc, b.len(), "TODO: handle short writes");
            i.consume(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Fake {
        cur: Arc<Mutex<Cursor<Vec<u8>>>>,
    }
    impl Default for Fake {
        fn default() -> Self {
            Self {
                cur: Arc::new(Mutex::new(Cursor::new(Vec::new()))),
            }
        }
    }

    impl Write for Fake {
        fn write(&mut self, b: &[u8]) -> std::result::Result<usize, std::io::Error> {
            self.cur.lock().unwrap().write(b)
        }
        fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
            self.cur.lock().unwrap().flush()
        }
    }

    #[test]
    fn writer_sink() -> Result<()> {
        let (mut b, prev) = VectorSource::new(b"hello world".to_vec());
        b.work()?;
        let fake = Fake::default();
        let mut b = WriterSink::<u8>::new(prev, fake.clone());
        b.work()?;
        assert_eq!(fake.cur.lock().unwrap().get_ref(), b"hello world");
        Ok(())
    }
}
