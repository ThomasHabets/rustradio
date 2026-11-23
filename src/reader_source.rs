use std::io::Read;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, SampleEncoding};

fn reader_thread<R: Read + Send + 'static>(
    mut reader: R,
    tx: std::sync::mpsc::SyncSender<Result<Vec<u8>>>,
    stopped: &AtomicBool,
) {
    let mut buf = [0; 1024];
    while !stopped.load(Ordering::Relaxed) {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => match e.kind() {
                std::io::ErrorKind::Interrupted => continue,
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
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
        if let Err(e) = tx.send(Ok(buf[..n].to_vec())) {
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
/// Use a reader that returns periodically if the worker must stop promptly
/// when the block is dropped. An in-progress blocking read cannot be interrupted.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct ReaderSource<T: SampleEncoding> {
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
    rx: std::sync::mpsc::Receiver<Result<Vec<u8>>>,
    stopped: Arc<AtomicBool>,
}

impl<T: SampleEncoding> ReaderSource<T> {
    pub fn new<R: Read + Send + 'static>(reader: R) -> Result<(Self, ReadStream<T>)> {
        let (dst, r) = crate::stream::new_stream()?;
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stopped);
        std::thread::Builder::new()
            .name("ReaderSourceReader".to_string())
            .spawn(move || reader_thread(reader, tx, &worker_stop))?;
        Ok((
            Self {
                buf: Vec::new(),
                dst,
                rx,
                stopped,
            }
            .with_output_metadata(),
            r,
        ))
    }
}

impl<T: SampleEncoding> Drop for ReaderSource<T> {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

impl<T> Block for ReaderSource<T>
where
    T: SampleEncoding + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let size = T::ENCODED_SIZE.get();
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
                    if !self.buf.is_empty() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            format!(
                                "Reader ended with {} of {size} bytes of a sample",
                                self.buf.len()
                            ),
                        )
                        .into());
                    }
                    debug!("ReaderSource: input closed");
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
    fn dropping_idle_reader_source_stops_worker() -> Result<()> {
        struct IdleReader {
            started: std::sync::mpsc::Sender<()>,
            _lifetime: std::sync::mpsc::Sender<()>,
        }
        impl Read for IdleReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.started.send(());
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        let (started, reading) = std::sync::mpsc::channel();
        let (lifetime, stopped) = std::sync::mpsc::channel();
        let (source, _out) = ReaderSource::<u8>::new(IdleReader {
            started,
            _lifetime: lifetime,
        })?;
        reading.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(source);
        assert_eq!(
            stopped.recv_timeout(Duration::from_secs(2)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        );
        Ok(())
    }

    #[test]
    fn retries_interrupted_reads() -> Result<()> {
        struct InterruptedOnce(bool);
        impl Read for InterruptedOnce {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                if std::mem::replace(&mut self.0, false) {
                    Err(std::io::ErrorKind::Interrupted.into())
                } else {
                    Ok(0)
                }
            }
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        reader_thread(InterruptedOnce(true), tx, &AtomicBool::new(false));
        assert!(rx.recv().unwrap()?.is_empty());
        Ok(())
    }

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
        let o = prev.read_buf()?;
        assert_eq!(o.slice(), b"hello world");
        assert!(o.tags().len() == 0);
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
        let o = prev.read_buf()?;
        assert_eq!(o.slice(), &[0x44434241, 1]);
        assert!(o.tags().len() == 0);
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
        let o = prev.read_buf()?;
        assert_eq!(o.slice(), &v);
        assert!(o.tags().len() == 0);
        Ok(())
    }
}
