//! Send stream to raw file.
use std::io::BufWriter;
use std::io::Write;

use log::{debug, error};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, ReadStream};
use crate::{Error, Result, Sample};

/// File write mode.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Create a new file. Fail if file already exists.
    Create,

    /// Overwrite existing file, or create a new file if it doesn't exist.
    Overwrite,

    /// Append to existing file, or create a new file if it doesn't exist.
    Append,
}

/// Builder for file sink.
pub struct FileSinkBuilder<T: Sample> {
    filename: std::path::PathBuf,
    flush: bool,
    mode: Mode,
    _dummy: std::marker::PhantomData<T>,
}

impl<T: Sample> FileSinkBuilder<T> {
    /// Create new FileSinkBuilder.
    /// Mode defaults to Create.
    pub fn new<P: Into<std::path::PathBuf>>(filename: P) -> Self {
        Self {
            filename: filename.into(),
            flush: false,
            mode: Mode::Create,
            _dummy: std::marker::PhantomData,
        }
    }
    /// Set mode.
    pub fn mode(mut self, m: Mode) -> Self {
        self.mode = m;
        self
    }
    /// Set flush mode (flush after every write).
    #[must_use]
    pub fn flush(mut self, v: bool) -> Self {
        self.flush = v;
        self
    }
    /// Build the FileSink.
    pub fn build(self, src: ReadStream<T>) -> Result<FileSink<T>> {
        FileSink::new(src, self.filename, self.mode).map(|mut b| {
            b.flush = self.flush;
            b
        })
    }
}

/// Send stream to raw file.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FileSink<T: Sample> {
    f: BufWriter<std::fs::File>,
    #[rustradio(in)]
    src: ReadStream<T>,
    filename: std::path::PathBuf,
    flush: bool,
}

impl<T: Sample> FileSink<T> {
    /// Create new builder.
    pub fn builder<P: Into<std::path::PathBuf>>(filename: P) -> FileSinkBuilder<T> {
        FileSinkBuilder::new(filename)
    }
    /// Create new FileSink block.
    pub fn new<P: Into<std::path::PathBuf>>(
        src: ReadStream<T>,
        filename: P,
        mode: Mode,
    ) -> Result<Self> {
        let filename = filename.into();
        debug!("Opening sink {}", filename.display());
        let f = BufWriter::new(
            match mode {
                Mode::Create => std::fs::File::options()
                    .read(false)
                    .write(true)
                    .create_new(true)
                    .open(&filename),
                Mode::Overwrite => std::fs::File::create(&filename),
                Mode::Append => std::fs::File::options()
                    .read(false)
                    .append(true)
                    .open(&filename),
            }
            .map_err(|e| Error::file_io(e, &filename))?,
        );
        Ok(Self {
            f,
            src,
            filename,
            flush: false,
        })
    }

    /// Flush the write buffer.
    fn flush(&mut self) -> Result<()> {
        self.f
            .flush()
            .map_err(|e| Error::file_io(e, &self.filename))
    }
}

impl<T: Sample> Drop for FileSink<T> {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            error!(
                "FileSink: Failed to flush to {} on Drop: {e}",
                self.filename.display()
            );
        }
    }
}

impl<T> Block for FileSink<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
        let (i, _tags) = self.src.read_buf()?;
        let n = i.len();
        if n == 0 {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        let mut v = Vec::with_capacity(T::size() * n);
        i.iter().for_each(|s: &T| {
            v.extend(&s.serialize());
        });
        self.f
            .write_all(&v)
            .map_err(|e| Error::file_io(e, &self.filename))?;
        if self.flush {
            self.flush()?;
        }
        i.consume(n);
        Ok(BlockRet::Again)
    }
}

/// Builder for file sink.
pub struct NoCopyFileSinkBuilder<T> {
    filename: std::path::PathBuf,
    flush: bool,
    mode: Mode,
    _dummy: std::marker::PhantomData<T>,
}

impl<T> NoCopyFileSinkBuilder<T> {
    /// Create new FileSinkBuilder.
    /// Mode defaults to Create.
    pub fn new<P: Into<std::path::PathBuf>>(filename: P) -> Self {
        Self {
            filename: filename.into(),
            flush: false,
            mode: Mode::Create,
            _dummy: std::marker::PhantomData,
        }
    }
    /// Set mode.
    pub fn mode(mut self, m: Mode) -> Self {
        self.mode = m;
        self
    }
    /// Set flush mode (flush after every write).
    #[must_use]
    pub fn flush(mut self, v: bool) -> Self {
        self.flush = v;
        self
    }
    /// Build the FileSink.
    pub fn build(self, src: NCReadStream<T>) -> Result<NoCopyFileSink<T>> {
        NoCopyFileSink::new(src, self.filename, self.mode).map(|mut b| {
            b.flush = self.flush;
            b
        })
    }
}

/// Send stream to raw file.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct NoCopyFileSink<T> {
    f: BufWriter<std::fs::File>,
    #[rustradio(in)]
    src: NCReadStream<T>,
    filename: std::path::PathBuf,
    flush: bool,
}

impl<T> NoCopyFileSink<T> {
    /// Create new builder.
    pub fn builder<P: Into<std::path::PathBuf>>(filename: P) -> NoCopyFileSinkBuilder<T> {
        NoCopyFileSinkBuilder::new(filename)
    }
    /// Create new NoCopyFileSink block.
    pub fn new<P: Into<std::path::PathBuf>>(
        src: NCReadStream<T>,
        filename: P,
        mode: Mode,
    ) -> Result<Self> {
        let filename = filename.into();
        debug!("Opening sink {}", filename.display());
        let f = BufWriter::new(
            match mode {
                Mode::Create => std::fs::File::options()
                    .read(false)
                    .write(true)
                    .create_new(true)
                    .open(&filename),
                Mode::Overwrite => std::fs::File::create(&filename),
                Mode::Append => std::fs::File::options()
                    .read(false)
                    .append(true)
                    .open(&filename),
            }
            .map_err(|e| Error::file_io(e, &filename))?,
        );
        Ok(Self {
            f,
            src,
            filename,
            flush: false,
        })
    }

    /// Flush the write buffer.
    pub fn flush(&mut self) -> Result<()> {
        self.f
            .flush()
            .map_err(|e| Error::file_io(e, &self.filename))
    }
}

impl<T> Drop for NoCopyFileSink<T> {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            error!(
                "NoCopyFileSink: Failed to flush to {} on Drop: {e}",
                self.filename.display()
            );
        }
    }
}

impl<T> Block for NoCopyFileSink<T>
where
    T: Sample<Type = T> + std::fmt::Debug + Default,
{
    fn work(&mut self) -> Result<BlockRet> {
        if let Some((s, _tags)) = self.src.pop() {
            // TODO: write tags.
            //let s2 = format!["{:?}", s].into();
            let mut v = s.serialize();
            v.push(10); // Newline.
            self.f
                .write_all(&v)
                .map_err(|e| Error::file_io(e, &self.filename))?;
            if self.flush {
                self.flush()?;
            }
            Ok(BlockRet::Again)
        } else {
            Ok(BlockRet::WaitForStream(&self.src, 1))
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::{Complex, Float};

    #[test]
    fn fail_create() -> Result<()> {
        let ssrc = ReadStream::from_slice(&[1.0 as Float, 3.0, 2.14, -2.14]);
        assert!(FileSink::<Float>::new(ssrc, "/dev/null", Mode::Create).is_err());
        Ok(())
    }

    #[test]
    fn overwrite() -> Result<()> {
        let ssrc = ReadStream::from_slice(&[1.0 as Float, 3.0, 2.14, -2.14]);
        assert!(FileSink::<Float>::new(ssrc, "/dev/null", Mode::Overwrite).is_ok());
        // TODO: check that it's not open for append.
        Ok(())
    }

    #[test]
    fn append() -> Result<()> {
        let ssrc = ReadStream::from_slice(&[1.0 as Float, 3.0, 2.14, -2.14]);
        assert!(FileSink::<Float>::new(ssrc, "/dev/null", Mode::Append).is_ok());
        // TODO: check that it's open for append.
        Ok(())
    }

    #[test]
    fn sink_f32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin");
        {
            #[allow(clippy::approx_constant)]
            let ssrc = ReadStream::from_slice(&[1.0 as Float, 3.0, 3.14, -3.14]);
            let mut sink = FileSink::<Float>::new(ssrc, tmpfn.clone(), Mode::Create)?;
            sink.work()?;
            sink.flush()?;
        }
        let out = std::fs::read(tmpfn)?;
        assert_eq!(
            out,
            vec![
                0, 0, 128, 63, 0, 0, 64, 64, 195, 245, 72, 64, 195, 245, 72, 192
            ]
        );
        Ok(())
    }

    #[test]
    fn sink_c32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin");
        {
            #[allow(clippy::approx_constant)]
            let ssrc = ReadStream::from_slice(&[Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)]);
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
