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
use crate::Error;

/** PDU writer

This block takes PDUs (as Vec<u8>), and writes them to an output
directory, named as microseconds since epoch.
*/
pub struct PduWriter {
    src: Streamp<Vec<u8>>,
    dir: PathBuf,
}

impl PduWriter {
    /// Create new PduWriter that'll write to `dir`.
    pub fn new(src: Streamp<Vec<u8>>, dir: PathBuf) -> Self {
        Self { src, dir }
    }
}

impl Block for PduWriter {
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
            f.write_all(packet)?;
        }
        input.clear();
        Ok(BlockRet::Ok)
    }
}
