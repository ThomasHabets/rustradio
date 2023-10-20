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
        }
    }

    /// Get output stream.
    pub fn out(&self) -> Streamp<Vec<u8>> {
        self.dst.clone()
    }

    fn update_state(
        dst: Streamp<Vec<u8>>,
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

                    // TODO: why do I need to map this? Why do I get a
                    // BS compile error when I do:
                    //
                    // dst.lock()?.push(bytes);
                    dst.lock()
                        .map_err(|e| Error::new(&format!("not possible?: {:?}", e)))?
                        .push(bytes);
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
