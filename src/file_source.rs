//! Read stream from raw file.
use std::io::BufReader;
use std::io::Read;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use log::{debug, warn};

use crate::block::{Block, BlockRet};
use crate::stream::Stream;
use crate::{Error, Sample};

/// Read stream from raw file.
pub struct FileSource<T: Copy> {
    filename: String,
    f: BufReader<std::fs::File>,
    repeat: bool,
    buf: Vec<u8>,
    dst: Arc<Mutex<Stream<T>>>,
}

impl<T: Default + Copy> FileSource<T> {
    /// Create new FileSource block.
    pub fn new(filename: &str, repeat: bool) -> Result<Self> {
        let f = BufReader::new(std::fs::File::open(filename)?);
        debug!("Opening source {filename}");
        Ok(Self {
            filename: filename.to_string(),
            f,
            repeat,
            buf: Vec::new(),
            dst: Arc::new(Mutex::new(Stream::<T>::new())),
        })
    }
    pub fn out(&self) -> Arc<Mutex<Stream<T>>> {
        self.dst.clone()
    }
}

impl<T> Block for FileSource<T>
where
    T: Sample<Type = T> + Copy + std::fmt::Debug,
{
    fn block_name(&self) -> &'static str {
        "FileSource"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let sample_size = T::size();
        let have = self.buf.len() / sample_size;
        let want = self.dst.lock().unwrap().capacity();

        if have < want {
            let get = want - have;
            let mut buffer = vec![0; get * sample_size];
            let n = self
                .f
                .read(&mut buffer[..])
                .map_err(|e| -> anyhow::Error { e.into() })?;
            if n == 0 {
                warn!("EOF on {}. Repeat: {}", self.filename, self.repeat);
                return Ok(BlockRet::EOF);
            }
            self.buf.extend(&buffer[..n]);
        }

        let have = self.buf.len() / sample_size;

        let mut v = Vec::with_capacity(have);
        for i in (0..(have * sample_size)).step_by(sample_size) {
            v.push(T::parse(&self.buf[i..i + sample_size])?);
        }
        self.buf.drain(0..(have * sample_size));
        self.dst.lock().unwrap().write_slice(&v);
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
