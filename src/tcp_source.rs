use std::io::Read;

use anyhow::Result;
use log::warn;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{Error, Sample};

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::io::Write;

    use crate::Float;

    #[test]
    fn partials() -> Result<()> {
        let listener = std::net::TcpListener::bind("[::1]:0")?;
        let addr = listener.local_addr()?;
        std::thread::spawn(move || {
            eprintln!("waiting for connection");
            let (mut stream, _) = listener.accept().unwrap();
            eprintln!("connected");

            let data = [
                79u8, 97, 60, 75, 144, 84, 179, 71, 229, 154, 231, 74, 124, 211, 143, 74,
            ];

            let pos = 0;
            let n = 6;
            stream.write_all(&data[pos..n]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));

            let pos = pos + n;
            let n = 3;
            stream.write_all(&data[pos..(pos + n)]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));

            stream.write_all(&data[pos + n..]).unwrap();
        });
        let mut src: TcpSource<Float> = match addr {
            std::net::SocketAddr::V4(_) => panic!("Where did IPv4 come from?"),
            std::net::SocketAddr::V6(a) => {
                println!("Connecting to port {}", a.port());
                TcpSource::new("[::1]", a.port())?
            }
        };
        let mut is = InputStreams::new();
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_float());
        src.work(&mut is, &mut os)?;
        let res: Streamp<Float> = os.get(0).into();
        let want: VecDeque<Float> = [12345678.91817].into();
        assert_eq!(*res.borrow().data(), want, "first failed");

        src.work(&mut is, &mut os)?;
        assert_eq!(
            *res.borrow().data(),
            vec![12345678.91817, 91817.12345678],
            "second failed"
        );

        src.work(&mut is, &mut os)?;
        assert_eq!(
            *res.borrow().data(),
            vec![
                12345678.91817,
                91817.12345678,
                7589234.4712893,
                4712893.7589234
            ],
            "third failed"
        );

        Ok(())
    }
}

pub struct TcpSource<T> {
    stream: std::net::TcpStream,
    buf: Vec<u8>,
    _t: T,
}

impl<T: Default> TcpSource<T> {
    pub fn new(addr: &str, port: u16) -> Result<Self> {
        Ok(Self {
            stream: std::net::TcpStream::connect(format!("{addr}:{port}"))?,
            buf: Vec::new(),
            _t: T::default(), // TODO: remove ugly.
        })
    }
}

impl<T> Block for TcpSource<T>
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
    Streamp<T>: From<StreamType>,
{
    fn work(&mut self, _r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let o: Streamp<T> = Self::get_output(w, 0);
        let size = T::size();
        let mut buffer = vec![0; o.borrow().capacity()];
        let n = self
            .stream
            .read(&mut buffer[..])
            .map_err(|e| -> anyhow::Error { e.into() })?;
        if n == 0 {
            warn!("TCP connection closed?");
            return Ok(BlockRet::EOF);
        }
        let mut v = Vec::new();
        v.reserve(n / size + 1);

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
        o.borrow_mut().write(v.into_iter());
        Ok(BlockRet::Ok)
    }
}
