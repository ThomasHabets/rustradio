//! Read stream from raw file.
use std::io::BufReader;
use std::io::{Read, Seek};

use anyhow::Result;
use log::{debug, trace, warn};

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Error, Sample};

/// Read stream from raw file.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FileSource<T: Copy> {
    filename: String,
    f: BufReader<std::fs::File>,
    repeat: bool,
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Default + Copy> FileSource<T> {
    /// Create new FileSource block.
    pub fn new(filename: &str, repeat: bool) -> Result<(Self, ReadStream<T>)> {
        let f = BufReader::new(
            std::fs::File::open(filename)
                .map_err(|e| Error::new(&format!("Failed to open {}: {:?}", filename, e)))?,
        );
        debug!("Opening source {filename}");
        let (dst, dr) = crate::stream::new_stream();
        Ok((
            Self {
                filename: filename.to_string(),
                f,
                repeat,
                buf: Vec::new(),
                dst,
            },
            dr,
        ))
    }
}

impl<T> Block for FileSource<T>
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut o = self.dst.write_buf()?;
        let sample_size = T::size();
        let have = self.buf.len() / sample_size;
        let want = o.len();
        if want == 0 {
            trace!("FileSource: no space left in output stream");
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }

        if have < want {
            let get = want - have;
            let get_bytes = get * sample_size;
            let mut buffer = vec![0; get_bytes];
            let n = self
                .f
                .read(&mut buffer[..])
                .map_err(|e| -> anyhow::Error { e.into() })?;
            if n == 0 {
                warn!("EOF on {}. Repeat: {}", self.filename, self.repeat);
                if self.repeat {
                    self.f.seek(std::io::SeekFrom::Start(0))?;
                    // This is not quite the definition of "pending", but I
                    // wanted to get rid of Noop, and it'll do for now.
                    // TODO: loop instead.
                    return Ok(BlockRet::Pending);
                } else {
                    return Ok(BlockRet::EOF);
                }
            }
            if self.buf.is_empty() && (n % sample_size) == 0 {
                // Fast path when reading only whole samples.
                o.fill_from_iter(
                    buffer
                        .chunks_exact(sample_size)
                        .map(|d| T::parse(d).unwrap()),
                );
                trace!("FileSource: Produced {} in fast path", n / sample_size);
                o.produce(n / sample_size, &[]);
                return Ok(BlockRet::Ok);
            }
            self.buf.extend(&buffer[..n]);
        }

        let have = self.buf.len() / sample_size;
        if have == 0 {
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
        trace!("FileSource: Produced {}", n);
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}

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

        let (mut src, src_out) = FileSource::<Float>::new(&tmpfn, false)?;
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
        let tmpfn = tmpd.path().join("delme.bin").display().to_string();

        std::fs::write(
            &tmpfn,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 195, 245, 72, 64, 205, 204, 44, 192],
        )?;

        let (mut src, src_out) = FileSource::<Complex>::new(&tmpfn, false)?;
        src.work()?;

        let (res, _) = src_out.read_buf()?;
        #[allow(clippy::approx_constant)]
        let correct = vec![Complex::new(0.0, 0.0), Complex::new(3.14, -2.7)];
        assert_eq!(res.slice(), correct);
        Ok(())
    }
}
