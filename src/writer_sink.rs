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

    #[rustradio(default)]
    buf: Vec<u8>,
}

impl<T: Sample> WriterSink<T> {
    pub fn new<R: Write + Send + 'static>(src: ReadStream<T>, writer: R) -> Self {
        Self {
            writer: Box::new(writer),
            src,
            buf: Vec::new(),
        }
    }
}

impl<T> Block for WriterSink<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            if !self.buf.is_empty() {
                let rc = match self.writer.write(&self.buf) {
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    result => result?,
                };
                if rc == 0 {
                    return Err(
                        std::io::Error::new(std::io::ErrorKind::WriteZero, "WriterSink").into(),
                    );
                }
                self.buf.drain(..rc);
                if !self.buf.is_empty() {
                    return Ok(BlockRet::Pending);
                }
                continue;
            }

            let (i, _) = self.src.read_buf()?;
            if i.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            self.buf.reserve(T::size() * i.len());
            for sample in i.iter() {
                sample.serialize_into(&mut self.buf);
            }
            let n = i.len();
            i.consume(n);
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
    fn retries_interrupted_writes() -> Result<()> {
        struct InterruptedWriter {
            first: bool,
            output: Fake,
        }
        impl Write for InterruptedWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if std::mem::take(&mut self.first) {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.output.write(data)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.output.flush()
            }
        }
        let output = Fake::default();
        let mut sink = WriterSink::new(
            ReadStream::from_slice(b"hello"),
            InterruptedWriter {
                first: true,
                output: output.clone(),
            },
        );
        sink.work()?;
        assert_eq!(output.cur.lock().unwrap().get_ref(), b"hello");
        Ok(())
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
