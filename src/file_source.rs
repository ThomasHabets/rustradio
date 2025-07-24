//! Read stream from raw file.
use std::io::BufReader;
use std::io::{Read, Seek};

use log::{debug, trace};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Error, Repeat, Result, Sample};

/// FileSource builder.
pub struct FileSourceBuilder<T: Sample> {
    filename: std::path::PathBuf,
    repeat: Repeat,
    _dummy: std::marker::PhantomData<T>,
}

impl<T: Sample> FileSourceBuilder<T> {
    /// Create builder.
    pub fn new<P: Into<std::path::PathBuf>>(filename: P) -> Self {
        FileSourceBuilder {
            filename: filename.into(),
            repeat: Repeat::finite(1),
            _dummy: std::marker::PhantomData,
        }
    }
    /// Build the FileSource.
    pub fn build(self) -> Result<(FileSource<T>, ReadStream<T>)> {
        let (mut block, dst) = FileSource::new(self.filename)?;
        block.repeat(self.repeat);
        Ok((block, dst))
    }

    /// Repeat mode.
    #[must_use]
    pub fn repeat(mut self, r: Repeat) -> Self {
        self.repeat = r;
        self
    }
}

/// Read stream from raw file.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FileSource<T: Sample> {
    filename: std::path::PathBuf,
    f: BufReader<std::fs::File>,
    repeat: Repeat,
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample> FileSource<T> {
    /// Create builder.
    ///
    /// `u8` is a dummy type to make `FileSource::builder()` work. It'll work
    /// for any type, not just u8.
    pub fn builder<P: Into<std::path::PathBuf>>(filename: P) -> FileSourceBuilder<T> {
        FileSourceBuilder::<T>::new(filename)
    }
    /// Create new FileSource block.
    pub fn new<P: Into<std::path::PathBuf>>(filename: P) -> Result<(Self, ReadStream<T>)> {
        let filename = filename.into();
        let f = BufReader::new(
            std::fs::File::open(&filename).map_err(|e| Error::file_io(e, filename.clone()))?,
        );
        debug!("Opening source {}", filename.display());
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            Self {
                filename,
                f,
                repeat: Repeat::finite(1),
                buf: Vec::new(),
                dst,
            },
            dr,
        ))
    }
    /// Set repeat mode.
    pub fn repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }
}

impl<T> Block for FileSource<T>
where
    T: Sample<Type = T> + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet> {
        let mut o = self.dst.write_buf()?;
        let sample_size = T::size();
        let have = self.buf.len() / sample_size;
        let want = o.len();
        if want == 0 {
            trace!("FileSource: no space left in output stream. have={have} want={want}");
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }

        if have < want {
            let get = want - have;
            let get_bytes = get * sample_size;
            let mut buffer = vec![0; get_bytes];
            let n = self.f.read(&mut buffer[..])?;
            if n == 0 {
                debug!(
                    "EOF on {}. Repeat: {:?}",
                    self.filename.display(),
                    self.repeat
                );
                if self.repeat.again() {
                    self.f.seek(std::io::SeekFrom::Start(0))?;
                    return Ok(BlockRet::Again);
                }
                return Ok(BlockRet::EOF);
            }
            if self.buf.is_empty() && n.is_multiple_of(sample_size) {
                // Fast path when reading only whole samples.
                o.fill_from_iter(
                    buffer
                        .chunks_exact(sample_size)
                        .map(|d| T::parse(d).unwrap()),
                );
                trace!("FileSource: Produced {} in fast path", n / sample_size);
                o.produce(n / sample_size, &[]);
                return Ok(BlockRet::Again);
            }
            self.buf.extend(&buffer[..n]);
        }

        let have = self.buf.len() / sample_size;
        if have == 0 {
            // Don't have a full sample.
            return Ok(BlockRet::Pending);
        }

        // TODO: remove needless copy.
        let v = self
            .buf
            .chunks_exact(sample_size)
            .map(|d| T::parse(d))
            .collect::<Result<Vec<_>>>()?;
        self.buf.drain(0..(have * sample_size));
        let n = v.len();
        o.fill_from_iter(v);
        trace!("FileSource: Produced {n}");
        o.produce(n, &[]);
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Complex, Float};

    #[test]
    fn source_f32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin");

        std::fs::write(
            &tmpfn,
            vec![
                0, 0, 128, 63, 0, 0, 64, 64, 195, 245, 72, 64, 195, 245, 72, 192,
            ],
        )?;

        let (mut src, src_out) = FileSource::<Float>::new(&tmpfn)?;
        src.work()?;

        let (res, _) = src_out.read_buf()?;
        #[allow(clippy::approx_constant)]
        let correct = vec![1.0 as Float, 3.0, 3.14, -3.14];
        assert_eq!(res.slice(), correct);
        Ok(())
    }
    #[test]
    fn source_c32() -> Result<()> {
        let tmpd = tempfile::tempdir()?;
        let tmpfn = tmpd.path().join("delme.bin");

        std::fs::write(
            &tmpfn,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192],
        )?;

        let (mut src, src_out) = FileSource::<Complex>::new(&tmpfn)?;
        src.work()?;

        let (res, _) = src_out.read_buf()?;
        #[allow(clippy::approx_constant)]
        let correct = vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)];
        assert_eq!(res.slice(), correct);
        Ok(())
    }
}
