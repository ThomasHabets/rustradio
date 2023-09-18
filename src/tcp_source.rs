use anyhow::Result;
use log::warn;
use std::io::Read;

use crate::{Sample, Source, StreamWriter};

pub struct TcpSource {
    stream: std::net::TcpStream,
}

impl TcpSource {
    pub fn new(port: u16) -> Result<Self> {
        Ok(Self {
            stream: std::net::TcpStream::connect(format!("127.0.0.1:{port}"))?,
        })
    }
}

impl<T> Source<T> for TcpSource
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
{
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let mut buffer = vec![0; w.capacity()];
        let n = self.stream.read(&mut buffer[..])?;
        if n == 0 {
            warn!("TCP connection closed?");
            return Ok(());
        }
        let partial = n % T::size();
        if partial != 0 {
            let mut buf2 = vec![0; T::size() - partial];
            self.stream.read_exact(&mut buf2)?;
            buffer.extend(buf2);
        }
        let size = T::size();
        let mut v = Vec::new();
        for pos in (0..n).step_by(size) {
            v.push(T::parse(&buffer[pos..pos + size])?);
        }
        w.write(&v)
    }
}
