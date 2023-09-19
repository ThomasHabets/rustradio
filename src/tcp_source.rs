use anyhow::Result;
use log::warn;
use std::io::Read;

use crate::{Sample, Source, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_sink::VectorSink;
    use crate::{Float, Stream};
    use std::io::Write;

    #[test]
    fn partials() -> Result<()> {
        let mut s = Stream::new(10000);
        let mut sink: VectorSink<Float> = VectorSink::new();

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
        let mut src = match addr {
            std::net::SocketAddr::V4(_) => panic!("Where did IPv4 come from?"),
            std::net::SocketAddr::V6(a) => {
                println!("Connecting to port {}", a.port());
                TcpSource::new("[::1]", a.port())?
            }
        };
        src.work(&mut s)?;
        sink.work(&mut s)?;
        assert_eq!(sink.to_vec(), vec![12345678.91817], "first failed");

        src.work(&mut s)?;
        sink.work(&mut s)?;
        assert_eq!(
            sink.to_vec(),
            vec![12345678.91817, 91817.12345678],
            "second failed"
        );

        src.work(&mut s)?;
        sink.work(&mut s)?;
        assert_eq!(
            sink.to_vec(),
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

pub struct TcpSource {
    stream: std::net::TcpStream,
    buf: Vec<u8>,
}

impl TcpSource {
    pub fn new(addr: &str, port: u16) -> Result<Self> {
        Ok(Self {
            stream: std::net::TcpStream::connect(format!("{addr}:{port}"))?,
            buf: Vec::new(),
        })
    }
}

impl<T> Source<T> for TcpSource
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
{
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let size = T::size();
        let mut buffer = vec![0; w.capacity()];
        let n = self.stream.read(&mut buffer[..])?;
        if n == 0 {
            warn!("TCP connection closed?");
            return Ok(());
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
        w.write(&v)
    }
}
