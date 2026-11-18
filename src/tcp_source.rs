/*! TCP source.

Currently only implements TCP client mode.
*/
use std::io::{self, Read};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, SampleEncoding};

/// TCP Source, connecting to a server and streaming the data.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct TcpSource<T: SampleEncoding> {
    stream: std::net::TcpStream,
    buf: Vec<u8>,
    read_buffer: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: SampleEncoding> TcpSource<T> {
    /// Connect to an address (such as a `SocketAddr` or `(host, port)`) and
    /// create a TCP source block.
    pub fn new(
        addr: impl std::net::ToSocketAddrs + std::fmt::Debug,
    ) -> Result<(Self, ReadStream<T>)> {
        let stream = std::net::TcpStream::connect(&addr)
            .map_err(|e| crate::Error::wrap(e, format!("connecting TCP source to {addr:?}")))?;
        stream.set_nonblocking(true)?;
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((
            Self {
                stream,
                buf: Vec::new(),
                read_buffer: Vec::new(),
                dst,
            }
            .with_output_metadata(),
            dr,
        ))
    }
}

impl<T> Block for TcpSource<T>
where
    T: SampleEncoding + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut o = self.dst.write_buf()?;
        let size = T::ENCODED_SIZE.get();
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let buffer = &mut self.read_buffer;
        buffer.resize(o.len() * size, 0);
        let n = loop {
            match self.stream.read(buffer) {
                Ok(n) => break n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(BlockRet::Pending),
                Err(e) => return Err(e.into()),
            }
        };
        if n == 0 {
            if self.buf.is_empty() {
                return Ok(BlockRet::EOF);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "TCP stream ended with {} of {size} bytes of a sample",
                    self.buf.len()
                ),
            )
            .into());
        }
        let mut v = Vec::with_capacity(n / size + 1);

        // Handle case of buffer containing just a partial T.
        let mut steal = 0;
        if !self.buf.is_empty() {
            steal = (size - self.buf.len()).min(n);
            self.buf.extend(&buffer[0..steal]);
            if self.buf.len() < size {
                return Ok(BlockRet::Pending);
            }
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

    use std::io::Write;
    use std::sync::{Arc, Barrier};

    use crate::Float;

    fn read_samples(
        src: &mut TcpSource<Float>,
        out: &ReadStream<Float>,
        count: usize,
    ) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while out.read_buf()?.0.len() < count {
            assert!(!matches!(src.work()?, BlockRet::EOF), "unexpected TCP EOF");
            assert!(std::time::Instant::now() < deadline, "TCP source stalled");
            std::thread::yield_now();
        }
        Ok(())
    }

    #[test]
    fn idle_peer_and_truncated_sample() -> Result<()> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let (mut src, _out) = TcpSource::<Float>::new(listener.local_addr()?)?;
        let (mut peer, _) = listener.accept()?;
        assert!(matches!(src.work()?, BlockRet::Pending));
        peer.write_all(&[1, 2, 3])?;
        drop(peer);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let error = loop {
            match src.work() {
                Err(error) => break error,
                Ok(BlockRet::Again | BlockRet::Pending) => {
                    assert!(std::time::Instant::now() < deadline, "TCP source stalled");
                    std::thread::yield_now();
                }
                Ok(other) => panic!("expected truncated-sample error, got {other:?}"),
            }
        };
        assert!(error.to_string().contains("3 of 4 bytes"), "{error}");
        Ok(())
    }

    #[test]
    fn partials() -> Result<()> {
        let listener = std::net::TcpListener::bind("[::1]:0")?;
        let addr = listener.local_addr()?;
        let barrier = Arc::new(Barrier::new(2));
        let barrier2 = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let barrier = barrier2;
            eprintln!("waiting for connection");
            let (mut stream, _) = listener.accept().unwrap();
            eprintln!("connected");

            let data = [
                79u8, 97, 60, 75, 144, 84, 179, 71, 229, 154, 231, 74, 124, 211, 143, 74,
            ];

            let pos = 0;
            let n = 6;
            stream.write_all(&data[pos..n]).unwrap();
            barrier.wait();
            barrier.wait();

            let pos = pos + n;
            let n = 3;
            stream.write_all(&data[pos..(pos + n)]).unwrap();
            barrier.wait();
            stream.write_all(&data[pos + n..]).unwrap();
        });
        let (mut src, src_out) = TcpSource::<Float>::new(addr)?;
        barrier.wait();
        read_samples(&mut src, &src_out, 1)?;
        {
            let (res, _) = src_out.read_buf()?;
            let want: Vec<Float> = [12345678.91817].into();
            assert_eq!(res.slice(), want, "first failed");
        }
        barrier.wait();
        read_samples(&mut src, &src_out, 2)?;
        {
            let (res, _) = src_out.read_buf()?;
            assert_eq!(
                res.slice(),
                vec![12345678.91817, 91_817.125],
                "second failed"
            );
        }

        barrier.wait();
        read_samples(&mut src, &src_out, 4)?;
        {
            let (res, _) = src_out.read_buf()?;
            assert_eq!(
                res.slice(),
                vec![12345678.91817, 91_817.125, 7_589_234.5, 4712893.7589234],
                "third failed"
            );
        }

        Ok(())
    }
}
