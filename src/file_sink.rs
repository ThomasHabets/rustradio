//! Send stream to raw file.
use std::io::BufWriter;
use std::io::Write;

use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockName, BlockRet};
use crate::stream::{NoCopyStreamp, Streamp};
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
pub struct FileSink<T: Copy> {
    f: BufWriter<std::fs::File>,
    src: Streamp<T>,
}

impl<T: Copy> FileSink<T> {
    /// Create new FileSink block.
    pub fn new(src: Streamp<T>, filename: std::path::PathBuf, mode: Mode) -> Result<Self> {
        debug!("Opening sink {}", filename.display());
        let f = BufWriter::new(match mode {
            Mode::Create => std::fs::File::options()
                .read(false)
                .write(true)
                .create_new(true)
                .open(filename)?,
            Mode::Overwrite => std::fs::File::create(filename)?,
            Mode::Append => std::fs::File::options()
                .read(false)
                .append(true)
                .open(filename)?,
        });
        Ok(Self { f, src })
    }

    /// Flush the write buffer.
    pub fn flush(&mut self) -> Result<()> {
        Ok(self.f.flush()?)
    }
}

impl<T> BlockName for FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn block_name(&self) -> &str {
        "FileSink"
    }
}
impl<T> Block for FileSink<T>
where
    T: Copy + Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _tags) = self.src.read_buf()?;
        let n = i.len();
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut v = Vec::with_capacity(T::size() * n);
        i.iter().for_each(|s: &T| {
            v.extend(&s.serialize());
        });
        self.f.write_all(&v)?;
        self.f.flush()?;
        i.consume(n);
        Ok(BlockRet::Ok)
    }
}

/// Send stream to raw file.
pub struct NoCopyFileSink<T> {
    f: BufWriter<std::fs::File>,
    src: NoCopyStreamp<T>,
}

impl<T> NoCopyFileSink<T> {
    /// Create new NoCopyFileSink block.
    pub fn new(src: NoCopyStreamp<T>, filename: std::path::PathBuf, mode: Mode) -> Result<Self> {
        debug!("Opening sink {}", filename.display());
        let f = BufWriter::new(match mode {
            Mode::Create => std::fs::File::options()
                .read(false)
                .write(true)
                .create_new(true)
                .open(filename)?,
            Mode::Overwrite => std::fs::File::create(filename)?,
            Mode::Append => std::fs::File::options()
                .read(false)
                .append(true)
                .open(filename)?,
        });
        Ok(Self { f, src })
    }

    /// Flush the write buffer.
    pub fn flush(&mut self) -> Result<()> {
        Ok(self.f.flush()?)
    }
}

impl<T> BlockName for NoCopyFileSink<T>
where
    T: Sample<Type = T> + std::fmt::Debug + Default,
{
    fn block_name(&self) -> &str {
        "NoCopyFileSink"
    }
}
impl<T> Block for NoCopyFileSink<T>
where
    T: Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        if let Some((s, _tags)) = self.src.pop() {
            // TODO: write tags.
            //let s2 = format!["{:?}", s].into();
            let mut v = s.serialize();
            v.push(10); // Newline.
            self.f.write_all(&v)?;
            self.f.flush()?;
            Ok(BlockRet::Ok)
        } else {
            Ok(BlockRet::Noop)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Complex, Float, Stream};

    #[test]
    fn sink_f32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin");
        {
            #[allow(clippy::approx_constant)]
            let ssrc = Stream::fromp_slice(&[1.0 as Float, 3.0, 3.14, -3.14]);
            let mut sink = FileSink::<Float>::new(ssrc, tmpfn.clone(), Mode::Create)?;
            sink.work()?;
            sink.flush()?;
        }
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![0, 0, 128, 63, 0, 0, 64, 64, 195, 245, 72, 64, 195, 245, 72, 192]
        );
        Ok(())
    }

    #[test]
    fn sink_c32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin");
        {
            #[allow(clippy::approx_constant)]
            let ssrc = Stream::fromp_slice(&[Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)]);
            let mut sink = FileSink::<Complex>::new(ssrc, tmpfn.clone(), Mode::Create)?;
            sink.work()?;
            sink.flush()?;
        }
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192]
        );
        Ok(())
    }
}
