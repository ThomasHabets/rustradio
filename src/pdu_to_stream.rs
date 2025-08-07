//! PDU to stream.

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, Tag, WriteStream};
use crate::{Result, Sample};

/// PDU to stream block.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct PduToStream<T: Sample> {
    #[rustradio(in)]
    src: NCReadStream<Vec<T>>,
    #[rustradio(out)]
    dst: WriteStream<T>,
    #[rustradio(default)]
    buf: Vec<T>,
    #[rustradio(default)]
    tags: Vec<Tag>,
}

impl<T: Sample> Block for PduToStream<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            while self.buf.is_empty() {
                let Some((pdu, tags)) = self.src.pop() else {
                    return Ok(BlockRet::WaitForStream(&self.src, 1));
                };
                self.buf = pdu;
                self.tags = tags;
            }
            let mut o = self.dst.write_buf()?;
            let n = std::cmp::min(o.len(), self.buf.len());
            if n == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            o.slice()[..n].copy_from_slice(&self.buf[..n]);
            self.buf.drain(0..n);
            // TODO: add some tags.
            o.produce(n, &self.tags);
            self.tags.clear();
        }
    }
}
