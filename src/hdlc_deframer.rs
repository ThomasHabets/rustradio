/*! HDLC Deframer.

[HDLC][hdlc] is used here and there. Notably by [AX.25][ax25] and
therefore [APRS][aprs].

[hdlc]: https://en.wikipedia.org/wiki/High-Level_Data_Link_Control
[ax25]: https://en.wikipedia.org/wiki/AX.25
[aprs]: https://en.wikipedia.org/wiki/Automatic_Packet_Reporting_System
 */
use log::{debug, info};

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
use crate::{Error, Result};

enum State {
    /// Looking for flag pattern.
    Unsynced(u8),

    /// Flag pattern seen. Accumulating bits for packet.
    Synced((u8, Vec<u8>)),

    /// Six ones in a row seen. Check the final bit for a 0, and emit
    /// packet if so.
    FinalCheck(Vec<u8>),
}

/** HDLC Deframer block.

This block takes a stream of bits (as u8), and outputs any HDLC frames
found as Vec<u8>.

TODO: Check checksum, and only output packets that pass.
*/
pub struct HdlcDeframer {
    src: Streamp<u8>,
    dst: Streamp<Vec<u8>>,
    state: State,
    min_size: usize,
    max_size: usize,
    strip_checksum: bool,
}

impl HdlcDeframer {
    /// Create new HdlcDeframer.
    ///
    /// min_size and max_size is size in bytes.
    pub fn new(src: Streamp<u8>, min_size: usize, max_size: usize) -> Self {
        Self {
            src,
            dst: new_streamp(),
            min_size,
            max_size,
            state: State::Unsynced(0xff),
            strip_checksum: true,
        }
    }

    /// Get output stream.
    pub fn out(&self) -> Streamp<Vec<u8>> {
        self.dst.clone()
    }

    fn update_state(
        dst: Streamp<Vec<u8>>,
        strip_checksum: bool,
        state: &mut State,
        min_size: usize,
        max_size: usize,
        bit: u8,
    ) -> Result<State> {
        Ok(match state {
            State::Unsynced(v) => {
                let n = (*v >> 1) | (bit << 7);
                if n == 0x7e {
                    debug!("HdlcDeframer: Found flag!");
                    State::Synced((0, Vec::with_capacity(max_size)))
                } else {
                    State::Unsynced(n)
                }
            }
            State::Synced((ones, inbits)) => {
                let mut bits: Vec<u8> = Vec::new();
                // We can't move from `bits`, since it's only borrowed,
                // but we can swap its contents.
                std::mem::swap(&mut bits, inbits);
                if inbits.len() > max_size * 8 {
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

                if bits.len() % 8 != 0 {
                    debug!("HdlcDeframer: Packet len not multiple of 8: {}", bits.len());
                } else if bits.len() / 8 < min_size {
                    debug!("Packet too short: {} < {}", bits.len() / 8, min_size);
                } else {
                    let bytes: Vec<u8> = (0..bits.len())
                        .step_by(8)
                        .map(|i| bits2byte(&bits[i..i + 8]))
                        .collect();
                    info!("HdlcDeframer: Captured packet: {:0>2x?}", bytes);
                    let data = &bytes[..bytes.len() - 2];
                    let got_crc = u16::from_le_bytes(bytes[bytes.len() - 2..].try_into()?);
                    let crc = calc_crc(data);
                    if crc != got_crc {
                        panic!("want crc {:0>4x}, got {:0>4x}", crc, got_crc);
                    }

                    // TODO: why do I need to map this? Why do I get a
                    // BS compile error when I do:
                    //
                    // dst.lock()?.push(bytes);
                    if strip_checksum {
                        dst.lock()
                            .map_err(|e| Error::new(&format!("not possible?: {:?}", e)))?
                            .push(data.to_vec());
                    } else {
                        dst.lock()
                            .map_err(|e| Error::new(&format!("not possible?: {:?}", e)))?
                            .push(bytes);
                    }
                }

                // We may or may not have seen a valid packet, but we
                // did see a valid flag. So back to synced.
                State::Synced((0, Vec::with_capacity(max_size)))
            }
        })
    }
}

impl Block for HdlcDeframer {
    fn block_name(&self) -> &'static str {
        "HDLC Deframer"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        for bit in input.iter().copied() {
            self.state = Self::update_state(
                self.dst.clone(),
                self.strip_checksum,
                &mut self.state,
                self.min_size,
                self.max_size,
                bit,
            )?;
        }
        input.clear();
        Ok(BlockRet::Ok)
    }
}

// Turn 8 bits in LSB order into a byte.
fn bits2byte(data: &[u8]) -> u8 {
    assert!(data.len() == 8);
    data[7] << 7
        | data[6] << 6
        | data[5] << 5
        | data[4] << 4
        | data[3] << 3
        | data[2] << 2
        | data[1] << 1
        | data[0]
}

// Calculate checksum. Code ported from RFC1662.
fn calc_crc(data: &[u8]) -> u16 {
    let fcstab: Vec<u16> = vec![
        0x0000, 0x1189, 0x2312, 0x329b, 0x4624, 0x57ad, 0x6536, 0x74bf, 0x8c48, 0x9dc1, 0xaf5a,
        0xbed3, 0xca6c, 0xdbe5, 0xe97e, 0xf8f7, 0x1081, 0x0108, 0x3393, 0x221a, 0x56a5, 0x472c,
        0x75b7, 0x643e, 0x9cc9, 0x8d40, 0xbfdb, 0xae52, 0xdaed, 0xcb64, 0xf9ff, 0xe876, 0x2102,
        0x308b, 0x0210, 0x1399, 0x6726, 0x76af, 0x4434, 0x55bd, 0xad4a, 0xbcc3, 0x8e58, 0x9fd1,
        0xeb6e, 0xfae7, 0xc87c, 0xd9f5, 0x3183, 0x200a, 0x1291, 0x0318, 0x77a7, 0x662e, 0x54b5,
        0x453c, 0xbdcb, 0xac42, 0x9ed9, 0x8f50, 0xfbef, 0xea66, 0xd8fd, 0xc974, 0x4204, 0x538d,
        0x6116, 0x709f, 0x0420, 0x15a9, 0x2732, 0x36bb, 0xce4c, 0xdfc5, 0xed5e, 0xfcd7, 0x8868,
        0x99e1, 0xab7a, 0xbaf3, 0x5285, 0x430c, 0x7197, 0x601e, 0x14a1, 0x0528, 0x37b3, 0x263a,
        0xdecd, 0xcf44, 0xfddf, 0xec56, 0x98e9, 0x8960, 0xbbfb, 0xaa72, 0x6306, 0x728f, 0x4014,
        0x519d, 0x2522, 0x34ab, 0x0630, 0x17b9, 0xef4e, 0xfec7, 0xcc5c, 0xddd5, 0xa96a, 0xb8e3,
        0x8a78, 0x9bf1, 0x7387, 0x620e, 0x5095, 0x411c, 0x35a3, 0x242a, 0x16b1, 0x0738, 0xffcf,
        0xee46, 0xdcdd, 0xcd54, 0xb9eb, 0xa862, 0x9af9, 0x8b70, 0x8408, 0x9581, 0xa71a, 0xb693,
        0xc22c, 0xd3a5, 0xe13e, 0xf0b7, 0x0840, 0x19c9, 0x2b52, 0x3adb, 0x4e64, 0x5fed, 0x6d76,
        0x7cff, 0x9489, 0x8500, 0xb79b, 0xa612, 0xd2ad, 0xc324, 0xf1bf, 0xe036, 0x18c1, 0x0948,
        0x3bd3, 0x2a5a, 0x5ee5, 0x4f6c, 0x7df7, 0x6c7e, 0xa50a, 0xb483, 0x8618, 0x9791, 0xe32e,
        0xf2a7, 0xc03c, 0xd1b5, 0x2942, 0x38cb, 0x0a50, 0x1bd9, 0x6f66, 0x7eef, 0x4c74, 0x5dfd,
        0xb58b, 0xa402, 0x9699, 0x8710, 0xf3af, 0xe226, 0xd0bd, 0xc134, 0x39c3, 0x284a, 0x1ad1,
        0x0b58, 0x7fe7, 0x6e6e, 0x5cf5, 0x4d7c, 0xc60c, 0xd785, 0xe51e, 0xf497, 0x8028, 0x91a1,
        0xa33a, 0xb2b3, 0x4a44, 0x5bcd, 0x6956, 0x78df, 0x0c60, 0x1de9, 0x2f72, 0x3efb, 0xd68d,
        0xc704, 0xf59f, 0xe416, 0x90a9, 0x8120, 0xb3bb, 0xa232, 0x5ac5, 0x4b4c, 0x79d7, 0x685e,
        0x1ce1, 0x0d68, 0x3ff3, 0x2e7a, 0xe70e, 0xf687, 0xc41c, 0xd595, 0xa12a, 0xb0a3, 0x8238,
        0x93b1, 0x6b46, 0x7acf, 0x4854, 0x59dd, 0x2d62, 0x3ceb, 0x0e70, 0x1ff9, 0xf78f, 0xe606,
        0xd49d, 0xc514, 0xb1ab, 0xa022, 0x92b9, 0x8330, 0x7bc7, 0x6a4e, 0x58d5, 0x495c, 0x3de3,
        0x2c6a, 0x1ef1, 0x0f78,
    ];

    let mut fcs = 0xffffu16;
    for byte in data {
        let byte = *byte as u16;
        let ofs = ((fcs ^ byte) & 0xff) as usize;
        fcs = (fcs >> 8) ^ fcstab[ofs];
    }
    fcs ^ 0xffff
}
