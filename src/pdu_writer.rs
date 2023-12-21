/*! PDU Writer

Writes received PDUs to a directory, with files named according to
receive time.

TODO: in the future the file naming should be configurable.
*/
use anyhow::Result;
use log::{debug, info};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::block::{Block, BlockRet};
use crate::stream::NoCopyStreamp;
use crate::{Error, Sample};

/** PDU writer

This block takes PDUs (as Vec<u8>), and writes them to an output
directory, named as microseconds since epoch.
*/
pub struct PduWriter<T> {
    src: NoCopyStreamp<Vec<T>>,
    dir: PathBuf,
    files_written: usize,
}

impl<T> Drop for PduWriter<T> {
    fn drop(&mut self) {
        info!("PDU Writer: wrote {}", self.files_written);
    }
}

impl<T> PduWriter<T> {
    /// Create new PduWriter that'll write to `dir`.
    pub fn new(src: NoCopyStreamp<Vec<T>>, dir: PathBuf) -> Self {
        Self {
            src,
            dir,
            files_written: 0,
        }
    }
}

impl<T> Block for PduWriter<T>
where
    T: Sample,
{
    fn block_name(&self) -> &'static str {
        "PDU Writer"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let packet = match self.src.pop() {
            None => return Ok(BlockRet::Noop),
            Some(x) => x,
        };
        let name = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_micros()
            .to_string();
        let full = Path::new(&self.dir).join(name);
        debug!("Saving PDU to {:?}", full);
        let mut f = std::fs::File::create(full)?;
        let mut v = Vec::with_capacity(T::size() * packet.len());
        packet.iter().for_each(|s: &T| {
            v.extend(&s.serialize());
        });
        f.write_all(&v)?;
        self.files_written += 1;
        Ok(BlockRet::Ok)
    }
}
