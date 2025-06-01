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

/// FCS adder.
///
/// Takes a packet, and adds 16 bit CRC to it.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct FcsAdder {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
}

impl Block for FcsAdder {
    fn work(&mut self) -> Result<BlockRet> {
        loop {
            let Some((mut data, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let crc = crate::hdlc_deframer::calc_crc(&data);
            data.extend(&[(crc & 0xff) as u8, ((crc >> 8) & 0xff) as u8]);
            self.dst.push(data, tags);
        }
    }
}

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
    use crate::stream::new_nocopy_stream;

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

    #[test]
    fn fcs() -> Result<()> {
        let data = vec![
            0x82u8, 0xa0, 0xb4, 0x60, 0x60, 0x62, 0x60, 0x9a, 0x60, 0xa8, 0x90, 0x86, 0x40, 0xe5,
            0x03, 0xf0, 0x3a, 0x4d, 0x30, 0x54, 0x48, 0x43, 0x2d, 0x31, 0x20, 0x20, 0x3a, 0x68,
            0x65, 0x6c, 0x6c, 0x6f, 0x41,
        ];
        let want: Vec<_> = data.iter().copied().chain(vec![0x7d, 0xdc]).collect();

        let (tx, rx) = new_nocopy_stream();
        tx.push(data, &[]);
        let (mut b, out) = FcsAdder::new(rx);
        b.work()?;
        let (got, _) = out.pop().unwrap();
        assert_eq!(got, want);
        Ok(())
    }
}
