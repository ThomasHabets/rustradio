/*! HDLC Deframer.

[HDLC][hdlc] is used here and there. Notably by [AX.25][ax25] and
therefore [APRS][aprs].

[hdlc]: https://en.wikipedia.org/wiki/High-Level_Data_Link_Control
[ax25]: https://en.wikipedia.org/wiki/AX.25
[aprs]: https://en.wikipedia.org/wiki/Automatic_Packet_Reporting_System
 */
use log::{debug, info, trace};

use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{NCWriteStream, ReadStream, Tag, TagValue};

enum State {
    /// Looking for flag pattern.
    Unsynced(u8),

    /// Flag pattern seen. Accumulating bits for packet.
    Synced((u8, Vec<u8>)),

    /// Six ones in a row seen. Check the final bit for a 0, and emit
    /// packet if so.
    FinalCheck(Vec<u8>),
}

impl Default for State {
    fn default() -> Self {
        State::Unsynced(0xff)
    }
}

// Calculate CRC. If a bitflip helps the CRC match, then return the
// new data with the CRC.
//
// Return tuple of:
// * new data, if modified.
// * correct CRC.
// * true/false if a bit was flipped or not.
fn find_right_crc(data: &[u8], got: u16, fix_bits: bool) -> (Option<Vec<u8>>, u16, bool) {
    let crc = calc_crc(data);
    if got == crc {
        // Fast path: CRC matches.
        return (None, crc, false);
    }
    if !fix_bits {
        return (None, crc, false);
    }
    let mut copy = data.to_vec();
    for byte in 0..data.len() {
        for bit in 0..8 {
            let x = 1 << bit;
            copy[byte] ^= x;
            let crc = calc_crc(&copy);
            if crc == got {
                debug!("Fixed bitflip successfully");
                return (Some(copy), crc, true);
            }
            copy[byte] ^= x;
        }
    }
    for crcbit in 0..16 {
        let newcrc = got ^ (1 << crcbit);
        if newcrc == got {
            debug!("Fixed bitflip in CRC successfully");
            return (None, newcrc, true);
        }
    }
    (None, crc, false)
}

/** HDLC Deframer block.

This block takes a stream of bits (as u8), and outputs any HDLC frames
found as `Vec<u8>`.
*/
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct HdlcDeframer {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
    #[rustradio(default)]
    state: State,
    min_size: usize,
    max_size: usize,
    #[rustradio(default)]
    keep_checksum: bool,
    #[rustradio(default)]
    decoded: usize,
    #[rustradio(default)]
    crc_error: usize,
    #[rustradio(default)]
    bitfixed: usize,
    #[rustradio(default)]
    stream_pos: u64,
    #[rustradio(default)]
    fix_bits: bool,
}

impl Drop for HdlcDeframer {
    fn drop(&mut self) {
        info!(
            "HDLC Deframer: Decoded {} (incl {} bitfixes), CRC error {}",
            self.decoded, self.bitfixed, self.crc_error
        );
    }
}

impl HdlcDeframer {
    /// Set fix bits.
    pub fn set_fix_bits(&mut self, v: bool) {
        self.fix_bits = v;
    }

    /// Set whether to check/strip checksum
    pub fn set_keep_checksum(&mut self, val: bool) {
        self.keep_checksum = val;
    }

    fn update_state(&mut self, bit: u8, stream_pos: u64) -> Result<State> {
        Ok(match &mut self.state {
            State::Unsynced(v) => {
                let n = (*v >> 1) | (bit << 7);
                if n == 0x7e {
                    trace!("HdlcDeframer: Found flag!");
                    State::Synced((0, Vec::with_capacity(self.max_size)))
                } else {
                    State::Unsynced(n)
                }
            }
            State::Synced((ones, inbits)) => {
                let mut bits: Vec<u8> = Vec::new();
                // We can't move from `bits`, since it's only borrowed,
                // but we can swap its contents.
                std::mem::swap(&mut bits, inbits);
                if bits.len() > self.max_size * 8 {
                    return Ok(State::Unsynced(0xff));
                }
                if bit > 0 {
                    bits.push(1);
                    if *ones == 5 {
                        State::FinalCheck(bits)
                    } else {
                        State::Synced((*ones + 1, bits))
                    }
                } else if *ones == 5 {
                    trace!("discarding stuffed bit {bits:?}");
                    State::Synced((0, bits))
                } else {
                    bits.push(0);
                    State::Synced((0, bits))
                }
            }
            State::FinalCheck(inbits) => {
                let mut bits: Vec<u8> = Vec::new();
                // We can't move from `bits`, since it's only borrowed,
                // but we can swap its contents.
                std::mem::swap(&mut bits, inbits);
                if bit == 1 {
                    // 7 ones in a row is invalid. Discard what we've collected.
                    return Ok(State::Unsynced(0xff));
                }
                if bits.len() < 7 {
                    // Too short, not even zero bytes.
                    return Ok(State::Unsynced(0xff));
                }

                // Remove partial flag.
                bits.truncate(bits.len() - 7);

                if !bits.len().is_multiple_of(8) {
                    trace!(
                        "HdlcDeframer: Packet len not multiple of 8: {} {:?}",
                        bits.len(),
                        bits
                    );
                } else if bits.len() / 8 < self.min_size {
                    trace!("Packet too short: {} < {}", bits.len() / 8, self.min_size);
                } else {
                    let bytes: Vec<u8> = (0..bits.len())
                        .step_by(8)
                        .map(|i| bits2byte(&bits[i..i + 8]))
                        .collect();
                    debug!("HdlcDeframer: Captured packet: {bytes:0>2x?}");
                    let tags = &[Tag::new(0, "packet_pos", TagValue::U64(stream_pos))];
                    if !self.keep_checksum {
                        let data = &bytes[..bytes.len() - 2];
                        let got_crc = u16::from_le_bytes(bytes[bytes.len() - 2..].try_into()?);
                        let (newdata, crc, fixed) = find_right_crc(data, got_crc, self.fix_bits);
                        if fixed {
                            self.bitfixed += 1;
                        }
                        let (data, crc) = match &newdata {
                            None => (data, crc),
                            Some(nd) => (&nd[..], crc),
                        };

                        if crc != got_crc {
                            self.crc_error += 1;
                            debug!("want crc {crc:0>4x}, got {got_crc:0>4x}");
                            return Ok(State::Synced((0, Vec::with_capacity(self.max_size))));
                        }
                        self.decoded += 1;
                        debug!("HdlcDeframer: Correctly decoded packet: {data:?}");
                        self.dst.push(data.to_vec(), tags);
                    } else {
                        self.decoded += 1;
                        self.dst.push(bytes, tags);
                    }
                }

                // We may or may not have seen a valid packet, but we
                // did see a valid flag. So back to synced.
                State::Synced((0, Vec::with_capacity(self.max_size)))
            }
        })
    }
}

impl Block for HdlcDeframer {
    fn work(&mut self) -> Result<BlockRet> {
        let (input, _tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        for bit in input.iter().copied() {
            // This is a bit ugly in that it destructively creates the
            // new state. The old state is moved from.
            self.state = self.update_state(bit, self.stream_pos)?;
            self.stream_pos += 1;
        }
        let n = input.len();
        input.consume(n);
        Ok(BlockRet::Again)
    }
}

// Turn 8 bits in LSB order into a byte.
fn bits2byte(data: &[u8]) -> u8 {
    assert!(data.len() == 8);
    (data[7] << 7)
        | (data[6] << 6)
        | (data[5] << 5)
        | (data[4] << 4)
        | (data[3] << 3)
        | (data[2] << 2)
        | (data[1] << 1)
        | data[0]
}

const FCSTAB: &[u16] = &[
    // End of line comments used to prevent fmt from creating too long lines.
    0x0000, 0x1189, 0x2312, 0x329b, 0x4624, 0x57ad, 0x6536, 0x74bf, 0x8c48, //.
    0x9dc1, 0xaf5a, 0xbed3, 0xca6c, 0xdbe5, 0xe97e, 0xf8f7, 0x1081, 0x0108, //.
    0x3393, 0x221a, 0x56a5, 0x472c, 0x75b7, 0x643e, 0x9cc9, 0x8d40, 0xbfdb, //.
    0xae52, 0xdaed, 0xcb64, 0xf9ff, 0xe876, 0x2102, 0x308b, 0x0210, 0x1399, //.
    0x6726, 0x76af, 0x4434, 0x55bd, 0xad4a, 0xbcc3, 0x8e58, 0x9fd1, 0xeb6e, //.
    0xfae7, 0xc87c, 0xd9f5, 0x3183, 0x200a, 0x1291, 0x0318, 0x77a7, 0x662e, //.
    0x54b5, 0x453c, 0xbdcb, 0xac42, 0x9ed9, 0x8f50, 0xfbef, 0xea66, 0xd8fd, //.
    0xc974, 0x4204, 0x538d, 0x6116, 0x709f, 0x0420, 0x15a9, 0x2732, 0x36bb, //.
    0xce4c, 0xdfc5, 0xed5e, 0xfcd7, 0x8868, 0x99e1, 0xab7a, 0xbaf3, 0x5285, //.
    0x430c, 0x7197, 0x601e, 0x14a1, 0x0528, 0x37b3, 0x263a, 0xdecd, 0xcf44, //.
    0xfddf, 0xec56, 0x98e9, 0x8960, 0xbbfb, 0xaa72, 0x6306, 0x728f, 0x4014, //.
    0x519d, 0x2522, 0x34ab, 0x0630, 0x17b9, 0xef4e, 0xfec7, 0xcc5c, 0xddd5, //.
    0xa96a, 0xb8e3, 0x8a78, 0x9bf1, 0x7387, 0x620e, 0x5095, 0x411c, 0x35a3, //.
    0x242a, 0x16b1, 0x0738, 0xffcf, 0xee46, 0xdcdd, 0xcd54, 0xb9eb, 0xa862, //.
    0x9af9, 0x8b70, 0x8408, 0x9581, 0xa71a, 0xb693, 0xc22c, 0xd3a5, 0xe13e, //.
    0xf0b7, 0x0840, 0x19c9, 0x2b52, 0x3adb, 0x4e64, 0x5fed, 0x6d76, 0x7cff, //.
    0x9489, 0x8500, 0xb79b, 0xa612, 0xd2ad, 0xc324, 0xf1bf, 0xe036, 0x18c1, //.
    0x0948, 0x3bd3, 0x2a5a, 0x5ee5, 0x4f6c, 0x7df7, 0x6c7e, 0xa50a, 0xb483, //.
    0x8618, 0x9791, 0xe32e, 0xf2a7, 0xc03c, 0xd1b5, 0x2942, 0x38cb, 0x0a50, //.
    0x1bd9, 0x6f66, 0x7eef, 0x4c74, 0x5dfd, 0xb58b, 0xa402, 0x9699, 0x8710, //.
    0xf3af, 0xe226, 0xd0bd, 0xc134, 0x39c3, 0x284a, 0x1ad1, 0x0b58, 0x7fe7, //.
    0x6e6e, 0x5cf5, 0x4d7c, 0xc60c, 0xd785, 0xe51e, 0xf497, 0x8028, 0x91a1, //.
    0xa33a, 0xb2b3, 0x4a44, 0x5bcd, 0x6956, 0x78df, 0x0c60, 0x1de9, 0x2f72, //.
    0x3efb, 0xd68d, 0xc704, 0xf59f, 0xe416, 0x90a9, 0x8120, 0xb3bb, 0xa232, //.
    0x5ac5, 0x4b4c, 0x79d7, 0x685e, 0x1ce1, 0x0d68, 0x3ff3, 0x2e7a, 0xe70e, //.
    0xf687, 0xc41c, 0xd595, 0xa12a, 0xb0a3, 0x8238, 0x93b1, 0x6b46, 0x7acf, //.
    0x4854, 0x59dd, 0x2d62, 0x3ceb, 0x0e70, 0x1ff9, 0xf78f, 0xe606, 0xd49d, //.
    0xc514, 0xb1ab, 0xa022, 0x92b9, 0x8330, 0x7bc7, 0x6a4e, 0x58d5, 0x495c, //.
    0x3de3, 0x2c6a, 0x1ef1, 0x0f78,
];

// Calculate checksum. Code ported from RFC1662.
#[must_use]
pub(crate) fn calc_crc(data: &[u8]) -> u16 {
    data.iter().fold(0xffffu16, |fcs, byte| {
        let byte = *byte as u16;
        let ofs = ((fcs ^ byte) & 0xff) as usize;
        (fcs >> 8) ^ FCSTAB[ofs]
    }) ^ 0xffff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::ReadStream;

    fn str2bits(s: &str) -> Vec<u8> {
        s.chars()
            .map(|ch| match ch {
                '1' => 1,
                '0' => 0,
                _ => panic!("invalid bitstring: {s}"),
            })
            .collect::<Vec<_>>()
    }

    #[test]
    fn find_simple_frame() -> Result<()> {
        //                12345678123456781234567812345678
        for bits in &[
            "01111110010101011110000001111110",
            "0101011111100101010111100000011111100101",
            "01111110011111100101011111100101010111100000011111100101",
            "01111110010101011110000001111110",
        ] {
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 1, 10);
            b.set_keep_checksum(true);
            b.work()?;
            let (res, _) = o.pop().unwrap();
            assert_eq!(res, vec![0xaa, 0x7]);
            assert!(o.pop().is_none());
        }
        Ok(())
    }
    #[test]
    fn find_simple_frames() -> Result<()> {
        for bits in &[
            "01111110010101011110000001111110010101011010101001111110",
            // One flag each.
            "0111111001010101111000000111111001111110010101011010101001111110",
            // One flag each, with garbage in between.
            &("01111110010101011110000001111110".to_owned()
                + "01011"
                + "01111110010101011010101001111110"),
        ] {
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 1, 10);
            b.set_keep_checksum(true);
            b.work()?;
            let (t, _tags) = o.pop().unwrap();
            assert_eq!(t, vec![0xaa, 0x7]);
            let (t, _tags) = o.pop().unwrap();
            assert_eq!(t, vec![0xaa, 0x55]);
            assert!(o.pop().is_none());
        }
        Ok(())
    }
    #[test]
    fn bitstuffed1() -> Result<()> {
        {
            let bits = &"01111110111110111110111110101111110";
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 1, 10);
            b.set_keep_checksum(true);
            b.work()?;
            let (res, _tags) = o.pop().unwrap();
            assert_eq!(res, vec![0xff, 0xff]);
            assert!(o.pop().is_none());
        }
        Ok(())
    }
    #[test]
    fn bitstuffed2() -> Result<()> {
        {
            let bits = &"01111110111110111110111110101111110";
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 1, 10);
            b.set_keep_checksum(true);
            b.work()?;
            let (res, _tags) = o.pop().unwrap();
            assert_eq!(res, vec![0xff, 0xff]);
        }
        Ok(())
    }
    #[test]
    fn too_short() -> Result<()> {
        {
            let bits = &"01111110111110111110111110101111110";
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 3, 10);
            b.set_keep_checksum(true);
            b.work()?;
            let res = o.pop();
            assert!(res.is_none(), "expected to discard short packet: {res:?}");
        }
        Ok(())
    }
    #[test]
    fn too_long() -> Result<()> {
        {
            let bits = &"01111110111110111110111110101111110";
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 1, 1);
            b.set_keep_checksum(true);
            b.work()?;
            let res = o.pop();
            assert!(res.is_none(), "expected to discard long packet: {res:?}");
        }
        Ok(())
    }
    #[test]
    fn check_crc() -> Result<()> {
        {
            let bits = &"0111111010101010000010101010111101111110";
            let s = ReadStream::from_slice(&str2bits(bits));
            let (mut b, o) = HdlcDeframer::new(s, 1, 10);
            b.work()?;
            let (res, _tags) = o.pop().unwrap();
            assert_eq!(res, vec![0x55]);
            assert!(o.pop().is_none());
        }
        Ok(())
    }
}
