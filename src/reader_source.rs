use std::io::Read;

use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, Sample};

fn reader_thread<R: Read + Send + 'static>(
    mut reader: R,
    tx: std::sync::mpsc::SyncSender<Result<Vec<u8>>>,
) {
    loop {
        let mut buf = vec![0; 1024];
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => match e.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                    // TODO: sleep?
                    continue;
                }
                _ => {
                    if let Err(e) = tx.send(Err(e.into())) {
                        debug!("ReaderSource reader thread failed to inform about read error: {e}");
                    }
                    return;
                }
            },
        };
        buf.truncate(n);
        if let Err(e) = tx.send(Ok(buf)) {
            debug!("ReaderSource reader thread failed to send data: {e}");
            return;
        }
        if n == 0 {
            // EOF.
            return;
        }
    }
}

/// Arbitrary reader source.
///
/// The underlying reader must periodically time out, or the graph will block on
/// e.g. Ctrl-C.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct ReaderSource<T: Sample> {
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
    rx: std::sync::mpsc::Receiver<Result<Vec<u8>>>,
}

impl<T: Sample> ReaderSource<T> {
    pub fn new<R: Read + Send + 'static>(reader: R) -> Result<(Self, ReadStream<T>)> {
        let (dst, r) = crate::stream::new_stream();
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        std::thread::Builder::new()
            .name("ReaderSourceReader".to_string())
            .spawn(move || reader_thread(reader, tx))?;
        Ok((
            Self {
                buf: Vec::new(),
                dst,
                rx,
            },
            r,
        ))
    }
}

impl<T> Block for ReaderSource<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
        let size = T::size();
        loop {
            let mut o = self.dst.write_buf()?;
            let ospace = o.len();
            if ospace == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            while self.buf.len() < size {
                let Ok(buf) = self.rx.try_recv() else {
                    return Ok(BlockRet::Pending);
                };
                let buf = buf?;
                if buf.is_empty() {
                    eprintln!("ReaderSource: Input closed");
                    return Ok(BlockRet::EOF);
                }
                self.buf.extend(buf);
            }
            let n = self.buf.len();
            let on = std::cmp::min(ospace, n / size);
            assert_ne!(on, 0);
            let n = on * size;
            for (pos, ov) in (0..n).step_by(size).zip(o.slice().iter_mut()) {
                *ov = T::parse(&self.buf[pos..pos + size])?;
            }
            self.buf.drain(..n);
            o.produce(on, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_source() -> Result<()> {
        let data = b"hello world";
        let r = std::io::Cursor::new(data);
        let (mut b, prev) = ReaderSource::<u8>::new(r)?;
        loop {
            if let Ok(BlockRet::EOF) = b.work() {
                break;
            }
        }
        let (o, tags) = prev.read_buf()?;
        assert_eq!(o.slice(), b"hello world");
        assert!(tags.is_empty());
        Ok(())
    }

    #[test]
    fn ints() -> Result<()> {
        let data = b"\x41\x42\x43\x44\x01\x00\x00\x00";
        let r = std::io::Cursor::new(data);
        let (mut b, prev) = ReaderSource::<u32>::new(r)?;
        loop {
            if let Ok(BlockRet::EOF) = b.work() {
                break;
            }
        }
        let (o, tags) = prev.read_buf()?;
        assert_eq!(o.slice(), &[0x44434241, 1]);
        assert!(tags.is_empty());
        Ok(())
    }

    #[test]
    fn big() -> Result<()> {
        let v: Vec<u32> = (0..2000).collect();
        let mut bytes = Vec::with_capacity(v.len() * 4);

        for num in &v {
            bytes.extend_from_slice(&num.to_le_bytes());
        }
        let r = std::io::Cursor::new(bytes);
        let (mut b, prev) = ReaderSource::<u32>::new(r)?;
        loop {
            if let Ok(BlockRet::EOF) = b.work() {
                break;
            }
        }
        let (o, tags) = prev.read_buf()?;
        assert_eq!(o.slice(), &v);
        assert!(tags.is_empty());
        Ok(())
    }
}
