use anyhow::Result;
use log::{debug, warn};
use std::io::Read;

use crate::{Sample, Source, StreamWriter};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_sink::VectorSink;
    use crate::{Complex, Float, Sink, Stream};

    #[test]
    fn source_f32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();

        std::fs::write(
            &tmpfn,
            vec![
                0, 0, 128, 63, 0, 0, 64, 64, 195, 245, 72, 64, 195, 245, 72, 192,
            ],
        )?;

        let mut src = FileSource::new(&tmpfn, false)?;
        let mut sink: VectorSink<Float> = VectorSink::new();
        let mut s = Stream::new(100);
        src.work(&mut s)?;
        sink.work(&mut s)?;

        #[allow(clippy::approx_constant)]
        let correct = vec![1.0 as Float, 3.0, 3.14, -3.14];
        assert_eq!(sink.to_vec(), correct);
        Ok(())
    }

    #[test]
    fn source_c32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();

        std::fs::write(
            &tmpfn,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192],
        )?;

        let mut src = FileSource::new(&tmpfn, false)?;
        let mut sink: VectorSink<Complex> = VectorSink::new();
        let mut s = Stream::new(100);
        src.work(&mut s)?;
        sink.work(&mut s)?;
        #[allow(clippy::approx_constant)]
        let correct = vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)];
        assert_eq!(sink.to_vec(), correct);
        Ok(())
    }
}

pub struct FileSource {
    filename: String,
    f: std::fs::File,
    repeat: bool,
    buf: Vec<u8>,
}

impl FileSource {
    pub fn new(filename: &str, repeat: bool) -> Result<Self> {
        let f = std::fs::File::open(filename)?;
        debug!("Opening source {filename}");
        Ok(Self {
            filename: filename.to_string(),
            f,
            repeat,
            buf: Vec::new(),
        })
    }
}

impl<T> Source<T> for FileSource
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
{
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        let mut buffer = vec![0; w.capacity()];
        let n = self.f.read(&mut buffer[..])?;
        if n == 0 {
            warn!(
                "Not handling EOF on {}. Repeat: {}",
                self.filename, self.repeat
            );
        }
        self.buf.extend(&buffer[..n]);

        let size = T::size();
        let samples = self.buf.len() / size;
        let mut v = Vec::new();
        for i in (0..(samples * size)).step_by(size) {
            v.push(T::parse(&self.buf[i..i + size])?);
        }
        self.buf.drain(0..(samples * size));
        w.write(&v)
    }
}
