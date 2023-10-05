//! Send stream to raw file.
use std::io::Write;

use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::{Error, Sample};

/// File write mode.
pub enum Mode {
    /// Create a new file. Fail if file already exists.
    Create,

    /// Overwrite existing file, or create a new file if it doesn't exist.
    Overwrite,

    /// Append to existing file, or create a new file if it doesn't exist.
    Append,
}

/// Send stream to raw file.
pub struct FileSink<T> {
    f: std::fs::File,
    dummy: std::marker::PhantomData<T>,
}

impl<T> FileSink<T> {
    /// Create new FileSink block.
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
        Ok(Self {
            f,
            dummy: std::marker::PhantomData,
        })
    }
}

impl<T> Block for FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
    Streamp<T>: From<StreamType>,
{
    fn block_name(&self) -> &'static str {
        "FileSink"
    }
    fn work(&mut self, r: &mut InputStreams, _w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let n = r.available(0);
        let mut v = Vec::with_capacity(T::size() * n);
        r.get(0).borrow().iter().for_each(|s: &T| {
            v.extend(&s.serialize());
        });
        self.f.write_all(&v)?;
        r.get(0).borrow_mut().consume(n);
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Complex, Float};

    #[test]
    fn sink_f32() -> Result<()> {
        #[allow(clippy::approx_constant)]
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();
        let mut sink = FileSink::<Float>::new(&tmpfn, Mode::Create)?;
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_float(&[1.0 as Float, 3.0, 3.14, -3.14]));
        sink.work(&mut is, &mut OutputStreams::new())?;
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
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();
        let mut sink = FileSink::<Complex>::new(&tmpfn, Mode::Create)?;
        let mut is = InputStreams::new();
        is.add_stream(StreamType::from_complex(&[
            Complex::new(0.0, 0.0),
            Complex::new(3.14, -2.7),
        ]));
        sink.work(&mut is, &mut OutputStreams::new())?;
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192]
        );
        Ok(())
    }
}
