use anyhow::Result;
use log::debug;
use std::io::Write;

use crate::{Sample, Sink, StreamReader};

mod tests {
    // These warnings about unused stuff are incorrect.
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use crate::vector_source::VectorSource;
    #[allow(unused_imports)]
    use crate::{Complex, Float, Stream};

    #[test]
    fn sink_f32() -> Result<()> {
        let mut src = VectorSource::new(vec![1.0 as Float, 3.0, 3.14, -3.14]);

        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();
        let mut sink = FileSink::new(tmpfn.clone(), Mode::Create)?;
        let mut s = Stream::new(10);
        src.work(&mut s)?;
        sink.work(&mut s)?;
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![63, 128, 0, 0, 64, 64, 0, 0, 64, 72, 245, 195, 192, 72, 245, 195]
        );
        Ok(())
    }

    #[test]
    fn sink_c32() -> Result<()> {
        let mut src = VectorSource::new(vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)]);

        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();
        let mut sink = FileSink::new(tmpfn.clone(), Mode::Create)?;
        let mut s = Stream::new(10);
        src.work(&mut s)?;
        sink.work(&mut s)?;
        eprintln!("tmpf: {tmpfn}");
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 64, 72, 245, 195, 192, 44, 204, 205],
        );
        Ok(())
    }
}

pub enum Mode {
    Create,
    Overwrite,
    Append,
}

pub struct FileSink<T> {
    _t: T, // TODO: remove this dummy.
    f: std::fs::File,
}

impl<T> FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    pub fn new(filename: String, mode: Mode) -> Result<Self> {
        let f = match mode {
            Mode::Create => std::fs::File::create(&filename)?, // TODO: don't overwrite.
            Mode::Overwrite => std::fs::File::create(&filename)?,
            Mode::Append => {
                todo!()
            }
        };
        debug!("Opening sink {filename}");
        Ok(Self {
            f,
            _t: T::default(),
        })
    }
}

impl<T> Sink<T> for FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self, r: &mut dyn StreamReader<T>) -> Result<()> {
        for s in r.buffer() {
            self.f.write_all(&s.serialize())?;
        }
        Ok(())
    }
}
