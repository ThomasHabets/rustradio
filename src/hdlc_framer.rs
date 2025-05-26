//! HDLC Framer.
//!
//! [HDLC][hdlc] is used here and there. Notably by [AX.25][ax25] and
//! therefore [APRS][aprs].
//!
//! [hdlc]: https://en.wikipedia.org/wiki/High-Level_Data_Link_Control
//! [ax25]: https://en.wikipedia.org/wiki/AX.25
//! [aprs]: https://en.wikipedia.org/wiki/Automatic_Packet_Reporting_System
use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream};

/// HDLC framer.
///
/// Takes a packet of bytes, and outputs a packet of bits.
///
/// It has to be a bunch of bits, because bit stuffing makes the output not
/// necessarily be byte aligned.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct HdlcFramer {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<bool>>,
}

impl Block for HdlcFramer {
    fn work(&mut self) -> Result<BlockRet> {
        let Some((x, tags)) = self.src.pop() else {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        };
        let _ = x;
        let _ = tags;
        let _ = &self.dst;
        todo!()
    }
}
