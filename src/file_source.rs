use anyhow::Result;
use log::{debug, warn};
use std::io::Read;

use crate::{Sample, Source, StreamWriter};

mod tests {
    // These warnings about unused stuff are incorrect.
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use crate::vector_sink::VectorSink;
    #[allow(unused_imports)]
    use crate::{Complex, Float, Stream};

    #[test]
    fn sink_f32() -> Result<()> {
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
        let mut s = Stream::new(10);
        src.work(&mut s)?;
        sink.work(&mut s)?;

        assert_eq!(sink.to_vec(), vec![1.0 as Float, 3.0, 3.14, -3.14]);
        Ok(())
    }

    #[test]
    fn sink_c32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();

        std::fs::write(
            &tmpfn,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192],
        )?;

        let mut src = FileSource::new(&tmpfn, false)?;
        let mut sink: VectorSink<Complex> = VectorSink::new();
        let mut s = Stream::new(10);
        src.work(&mut s)?;
        sink.work(&mut s)?;
        assert_eq!(
            sink.to_vec(),
            vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)]
        );
        Ok(())
    }
}

pub struct FileSource {
    filename: String,
    f: std::fs::File,
    repeat: bool,
}

impl FileSource {
    pub fn new(filename: &str, repeat: bool) -> Result<Self> {
        let f = std::fs::File::open(filename)?;
        debug!("Opening source {filename}");
        Ok(Self {
            filename: filename.to_string(),
            f,
            repeat,
        })
    }
}
impl<T> Source<T> for FileSource
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
{
    fn work(&mut self, w: &mut dyn StreamWriter<T>) -> Result<()> {
        // TODO: only read as much as w.capacity()
        let mut buffer = Vec::new();
        self.f.read_to_end(&mut buffer)?;
        let n = buffer.len();
        if n == 0 {
            warn!(
                "Not handling EOF on {}. Repeat: {}",
                self.filename, self.repeat
            );
        }

        let size = T::size();
        let mut v = Vec::new();
        for c in 0..(n / size) {
            let a = size * c;
            let b = a + size;
            v.push(T::parse(&buffer[a..b])?);
        }
        w.write(&v)
    }
}
