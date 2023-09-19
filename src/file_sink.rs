use anyhow::Result;
use log::debug;
use std::io::Write;

use crate::{Sample, Sink, StreamReader};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_source::VectorSource;
    use crate::{Complex, Float, Source, Stream};

    #[test]
    fn sink_f32() -> Result<()> {
        #[allow(clippy::approx_constant)]
        let mut src = VectorSource::new(vec![1.0 as Float, 3.0, 3.14, -3.14]);

        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();
        let mut sink = FileSink::new(&tmpfn, Mode::Create)?;
        let mut s = Stream::new(10);
        src.work(&mut s)?;
        sink.work(&mut s)?;
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![0, 0, 128, 63, 0, 0, 64, 64, 195, 245, 72, 64, 195, 245, 72, 192]
        );
        Ok(())
    }

    #[test]
    fn sink_c32() -> Result<()> {
        #[allow(clippy::approx_constant)]
        let mut src = VectorSource::new(vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)]);

        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();
        let mut sink = FileSink::new(&tmpfn, Mode::Create)?;
        let mut s = Stream::new(10);
        src.work(&mut s)?;
        sink.work(&mut s)?;
        eprintln!("tmpf: {tmpfn}");
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192]
        );
        Ok(())
    }
}

pub enum Mode {
    Create,
    Overwrite,
    Append,
}

pub struct FileSink {
    f: std::fs::File,
}

impl FileSink {
    pub fn new(filename: &str, mode: Mode) -> Result<Self> {
        let f = match mode {
            Mode::Create => std::fs::File::options()
                .read(false)
                .write(true)
                .create_new(true)
                .open(filename)?,
            Mode::Overwrite => std::fs::File::create(filename)?,
            Mode::Append => std::fs::File::options()
                .read(false)
                .write(true)
                .append(true)
                .open(filename)?,
        };
        debug!("Opening sink {filename}");
        Ok(Self { f })
    }
}

impl<T> Sink<T> for FileSink
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>) -> Result<()> {
        let mut v = Vec::new();
        v.reserve(T::size() * r.available());
        for s in r.buffer() {
            v.extend(&s.serialize());
        }
        self.f.write_all(&v)?;
        r.consume(r.available());
        Ok(())
    }
}
