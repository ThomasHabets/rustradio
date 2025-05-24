/*! TCP source.

Currently only implements TCP client mode.
*/
use std::io::Read;

use log::warn;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Result, Sample};

/// TCP Source, connecting to a server and streaming the data.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct TcpSource<T: Sample> {
    stream: std::net::TcpStream,
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample> TcpSource<T> {
    /// Create new TCP source block.
    pub fn new(addr: &str, port: u16) -> Result<(Self, ReadStream<T>)> {
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            Self {
                stream: std::net::TcpStream::connect(format!("{addr}:{port}"))?,
                buf: Vec::new(),
                dst,
            },
            dr,
        ))
    }
}

impl<T> Block for TcpSource<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
        let mut o = self.dst.write_buf()?;
        let size = T::size();
        let mut buffer = vec![0; o.len()];
        // TODO: this read blocks.
        let n = self.stream.read(&mut buffer[..])?;
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

    use std::io::Write;
    use std::sync::{Arc, Barrier};

    use crate::Float;

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
        let (mut src, src_out): (TcpSource<Float>, _) = match addr {
            std::net::SocketAddr::V4(_) => panic!("Where did IPv4 come from?"),
            std::net::SocketAddr::V6(a) => {
                println!("Connecting to port {}", a.port());
                TcpSource::new("[::1]", a.port())?
            }
        };
        barrier.wait();
        src.work()?;
        {
            let (res, _) = src_out.read_buf()?;
            let want: Vec<Float> = [12345678.91817].into();
            assert_eq!(res.slice(), want, "first failed");
        }
        barrier.wait();
        src.work()?;
        {
            let (res, _) = src_out.read_buf()?;
            assert_eq!(
                res.slice(),
                vec![12345678.91817, 91_817.125],
                "second failed"
            );
        }

        barrier.wait();
        src.work()?;
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
