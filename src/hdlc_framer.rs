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

const SYNC_BYTES: usize = 10;
const SYNC: &[bool] = &[false, true, true, true, true, true, true, false];

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

// TODO: confirm that I didn't get the bit order backwards.
fn hdlc_encode(data: &[u8]) -> Vec<bool> {
    let mut out = Vec::with_capacity(data.len() * 8 + 32);
    for _ in 0..SYNC_BYTES {
        out.extend(SYNC);
    }
    let mut ones = 0;
    for mut byte in data.iter().copied() {
        for _ in 0..8 {
            if byte & 1 == 1 {
                ones += 1;
                out.push(true);
                if ones == 5 {
                    ones = 0;
                    out.push(false);
                }
            } else {
                ones = 0;
                out.push(false);
            }
            byte >>= 1;
        }
    }
    for _ in 0..SYNC_BYTES {
        out.extend(SYNC);
    }
    out
}

impl Block for HdlcFramer {
    fn work(&mut self) -> Result<BlockRet> {
        loop {
            let Some((x, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let out = hdlc_encode(&x);
            self.dst.push(out, tags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bools_to_string(bs: &[bool]) -> String {
        let mut s = String::new();
        for b in bs {
            s += if *b { "1" } else { "0" };
        }
        s
    }

    #[test]
    fn various() {
        let pairs: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"\x00", "00000000"),
            (b"\x00\x55", "0000000010101010"),
            (b"\x00\xff\x55", "0000000011111011110101010"),
            (b"\x00\xff\xff", "000000001111101111101111101"),
            (b"\x00\xff\xff\xff", "000000001111101111101111101111101111"),
            (b"\xaa\x07", "0101010111100000"),
        ];
        for (i, o) in pairs {
            let want: Vec<_> = SYNC
                .repeat(SYNC_BYTES)
                .iter()
                .copied()
                .chain(o.chars().map(|b| b == '1'))
                .chain(SYNC.repeat(SYNC_BYTES).iter().copied())
                .collect();
            let got = hdlc_encode(&*i);
            assert_eq!(
                got,
                want,
                "\nwant: {}\ngot:  {}",
                bools_to_string(&want),
                bools_to_string(&got)
            );
        }
    }
}
