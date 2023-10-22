/*! PDU Writer

Writes received PDUs to a directory, with files named according to
receive time.

TODO: in the future the file naming should be configurable.
*/
use anyhow::Result;
use log::debug;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::block::{Block, BlockRet};
use crate::stream::Streamp;
use crate::{Error, Sample};

/** PDU writer

This block takes PDUs (as Vec<u8>), and writes them to an output
directory, named as microseconds since epoch.
*/
pub struct PduWriter<T> {
    src: Streamp<Vec<T>>,
    dir: PathBuf,
}

impl<T> PduWriter<T> {
    /// Create new PduWriter that'll write to `dir`.
    pub fn new(src: Streamp<Vec<T>>, dir: PathBuf) -> Self {
        Self { src, dir }
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
        let mut input = self.src.lock()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        for packet in input.iter() {
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
        }
        input.clear();
        Ok(BlockRet::Ok)
    }
}
