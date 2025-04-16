/*! PDU Writer

Writes received PDUs to a directory, with files named according to
receive time.

TODO: in the future the file naming should be configurable.
*/
use crate::Result;
use log::{debug, info};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::Sample;
use crate::block::{Block, BlockRet};
use crate::stream::NCReadStream;

/** PDU writer

This block takes PDUs (as `Vec<u8>`), and writes them to an output
directory, named as microseconds since epoch.
*/
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct PduWriter<T> {
    #[rustradio(in)]
    src: NCReadStream<Vec<T>>,
    #[rustradio(into)]
    dir: PathBuf,
    #[rustradio(default)]
    files_written: usize,
}

impl<T> Drop for PduWriter<T> {
    fn drop(&mut self) {
        info!("PDU Writer: wrote {}", self.files_written);
    }
}

impl<T> Block for PduWriter<T>
where
    T: Sample,
{
    fn work(&mut self) -> Result<BlockRet> {
        let packet = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some((x, _tags)) => x,
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
        Ok(BlockRet::Again)
    }
}
