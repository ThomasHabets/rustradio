use anyhow::Result;
use log::{debug, warn};
use std::io::Read;

use crate::block::{get_output, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{Error, Sample};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Complex, Float};

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

        let mut src = FileSource::<Float>::new(&tmpfn, false)?;
        let mut is = InputStreams::new();
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_float());
        src.work(&mut is, &mut os)?;

        let res: Streamp<Float> = os.get(0).into();
        #[allow(clippy::approx_constant)]
        let correct = vec![1.0 as Float, 3.0, 3.14, -3.14];
        assert_eq!(*res.borrow().data(), correct);
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

        let mut src = FileSource::<Complex>::new(&tmpfn, false)?;
        let mut is = InputStreams::new();
        let mut os = OutputStreams::new();
        os.add_stream(StreamType::new_complex());
        src.work(&mut is, &mut os)?;

        let res: Streamp<Complex> = os.get(0).into();
        #[allow(clippy::approx_constant)]
        let correct = vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)];
        assert_eq!(*res.borrow().data(), correct);
        Ok(())
    }
}

pub struct FileSource<T> {
    filename: String,
    f: std::fs::File,
    repeat: bool,
    buf: Vec<u8>,
    _t: T,
}

impl<T: Default> FileSource<T> {
    pub fn new(filename: &str, repeat: bool) -> Result<Self> {
        let f = std::fs::File::open(filename)?;
        debug!("Opening source {filename}");
        Ok(Self {
            filename: filename.to_string(),
            f,
            repeat,
            buf: Vec::new(),
            _t: T::default(),
        })
    }
}

impl<T> Block for FileSource<T>
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
    Streamp<T>: From<StreamType>,
{
    fn work(&mut self, _r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let mut buffer = vec![0; w.capacity(0)];
        let n = self
            .f
            .read(&mut buffer[..])
            .map_err(|e| -> anyhow::Error { e.into() })?;
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
        get_output(w, 0).borrow_mut().write_slice(&v);
        Ok(BlockRet::Ok)
    }
}
