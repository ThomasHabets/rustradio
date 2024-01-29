/*! IL2P Deframer

*/
use log::info;

use crate::block::{Block, BlockRet};
use crate::stream::{Streamp, Tag};
use crate::{Error, Result};

const HEADER_SIZE: usize = 15 * 8;

struct Pids {}
impl Pids {
    pub const AX25_UNNUMBERED: u8 = 1;
}

/// LFSR as used by IL2P.
///
/// Input is XORed into the masked positions of the shift register,
/// and output is just the last bit in it.
///
/// Len is implied by seed and mask.
struct Lfsr {
    mask: u64,
    shift_reg: u64,
}

impl Lfsr {
    /// Create new LFSR.
    fn new(mask: u64, seed: u64) -> Self {
        Self {
            mask,
            shift_reg: seed,
        }
    }
    /// Clock the LFSR.
    fn next(&mut self, i: u8) -> u8 {
        assert!(i <= 1);
        let i = i & 1;
        let ret = 1 & (i ^ self.shift_reg as u8);
        self.shift_reg = (self.shift_reg >> 1) ^ (self.mask * i as u64);
        ret
    }
}

fn bits_to_bytes(bits: &[u8]) -> Vec<u8> {
    assert![bits.len() % 8 == 0];
    let mut bytes = vec![];
    for chunk in bits.chunks(8) {
        let mut byte = 0u8;
        for (i, bit) in chunk.iter().enumerate() {
            byte |= bit << (7 - i);
        }
        bytes.push(byte);
    }
    bytes
}

enum State {
    Unsynced,
    Header(Vec<u8>),
    //Data(Vec<u8>, usize),
}

/// IL2P deframer block
pub struct Il2pDeframer {
    src: Streamp<u8>,
    decoded: usize,
    state: State,
}

impl Drop for Il2pDeframer {
    fn drop(&mut self) {
        info!("IL2P Deframer: Decoded {}", self.decoded);
    }
}
impl Il2pDeframer {
    /// New
    pub fn new(src: Streamp<u8>) -> Self {
        Self {
            src,
            decoded: 0,
            state: State::Unsynced,
        }
    }
}

impl Block for Il2pDeframer {
    fn block_name(&self) -> &'static str {
        "IL2P Deframer"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (input, tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let tags: Vec<Tag> = tags.into_iter().filter(|t| t.key() == "sync").collect();

        // If we hit an unexpected error, then go back to our default state.
        let mut oldstate = State::Unsynced;
        std::mem::swap(&mut oldstate, &mut self.state);

        // TODO: support delivering the payload, too.
        let (header, newstate) = match oldstate {
            State::Unsynced => {
                if tags.is_empty() {
                    let n = input.len();
                    input.consume(n);
                    (None as Option<Result<Header>>, State::Unsynced)
                } else {
                    input.consume(tags[0].pos() + 1);
                    (None, State::Header(Vec::new()))
                }
            }
            State::Header(mut partial) => {
                let remaining = HEADER_SIZE - partial.len();
                let get = std::cmp::min(input.len(), remaining);
                for bit in input.iter().take(get) {
                    partial.push(*bit);
                }
                input.consume(get);
                assert_eq![remaining == get, partial.len() == HEADER_SIZE];
                if partial.len() == HEADER_SIZE {
                    let header_bytes = bits_to_bytes(&decode(&partial[..]));

                    // TODO: run FEC, instead of just stripping it off.
                    let header_bytes = &header_bytes[..header_bytes.len() - 2];

                    let header = Header::parse(header_bytes);
                    (Some(header), State::Unsynced)
                } else {
                    (None, State::Header(partial))
                }
            }
        };
        self.state = newstate;

        if let Some(Ok(header)) = header {
            info!("Got header");
            info!("  {:?}", &header);
            info!("  {} => {}", header.src, header.dst);
            info!("  control: 0x{:x}", header.control);
            info!("  describe: {}", header.describe());
            info!("  fec: {}", header.fec);
            info!("  payload_size: {}", header.payload_size);
        } else if let Some(Err(e)) = header {
            info!("Failed to parse header: {}", e);
        }
        Ok(BlockRet::Ok)
    }
}
/*
Reed solomon decoder with u8 as symbols, and two ecc symbols added to the end. Zero as the first root.
The Galois Field is defined by reducing polynomial x^8+x^4+x^3+x^2+1.

https://www.kernel.org/doc/html/v4.15/core-api/librs.html
https://berthub.eu/articles/posts/reed-solomon-for-programmers/
direwolf commit 53e9ff7908621307cd9d46d6f54f5a1e06102ff7
*/

fn decode(input: &[u8]) -> Vec<u8> {
    /*
    // RS parameters.
    let symbol_size = 8;
    let parity_size = 2;
    // poly: x^8+x^4+x^3+x^2+1.
    // primitive element field?
    let primitive_element = 1;
    let first_root = 0;
     */
    let mut l = Lfsr::new(0x108, 0x1f0);
    let mut ret = Vec::new();
    for bit in input {
        ret.push(l.next(*bit));
    }
    ret
}

fn decode_callsign(input: &[u8]) -> Result<String> {
    Ok(String::from_utf8(
        input
            .iter()
            .map(|ch| ch & 63)
            .filter(|ch| *ch > 0)
            .map(|ch| ch + 0x20)
            .collect(),
    )?)
}

#[derive(Debug)]
struct Header {
    dst: String,
    src: String,
    ui: bool,
    fec: bool,
    pid: u8,     // 4 bits
    control: u8, // 7 bits
    hdrtype1: bool,
    payload_size: u16, // 10 bits
}

impl Header {
    fn parse(data: &[u8]) -> Result<Self> {
        assert_eq!(data.len(), 13);
        Ok(Self {
            dst: format!("{}-{}", decode_callsign(&data[0..6])?, data[12] >> 4),
            src: format!("{}-{}", decode_callsign(&data[6..12])?, data[12] & 0xf),
            ui: (data[0] & 0x40) != 0,
            fec: (data[0] & 0x80) != 0,
            hdrtype1: (data[1] & 0x80) != 0,
            pid: (data[1] & 0x40) >> 3
                | (data[2] & 0x40) >> 4
                | (data[3] & 0x40) >> 5
                | (data[4] & 0x40) >> 6,
            control: (data[5] & 0x40)
                | (data[6] & 0x40) >> 1
                | (data[7] & 0x40) >> 2
                | (data[8] & 0x40) >> 3
                | (data[9] & 0x40) >> 4
                | (data[10] & 0x40) >> 5
                | (data[11] & 0x40) >> 6,
            payload_size: (data[2] as u16 & 0x80) << 2
                | (data[3] as u16 & 0x80) << 1
                | (data[4] as u16 & 0x80)
                | (data[5] as u16 & 0x80) >> 1
                | (data[6] as u16 & 0x80) >> 2
                | (data[7] as u16 & 0x80) >> 3
                | (data[8] as u16 & 0x80) >> 4
                | (data[9] as u16 & 0x80) >> 5
                | (data[10] as u16 & 0x80) >> 6
                | (data[11] as u16 & 0x80) >> 7,
        })
    }
    fn describe(&self) -> String {
        match self.hdrtype1 {
            true => match self.ui {
                false => match self.pid {
                    Pids::AX25_UNNUMBERED => match (self.control >> 2) & 0xF {
                        0x0 => "invalid 0x00".into(),
                        0x1 => "SABM".into(),
                        0x2 => "invalid 0x02".into(),
                        0x3 => "DISC".into(),
                        0x4 => "DM".into(),
                        0x5 => "invalid 0x05".into(),
                        0x6 => "UA".into(),
                        0x7 => "invalid 0x07".into(),
                        0x8 => "FRMR".into(),
                        0x9 => "unvalid 0x09".into(),
                        0xA => "UI unnumbered response".into(),
                        0xB => "UI unnumbered command".into(),
                        0xC => "XID response".into(),
                        0xD => "XID command".into(),
                        0xE => "TEST response".into(),
                        0xF => "TEST command".into(),
                        16.. => "Can't happen".into(),
                    },
                    _ => "other PID".into(),
                },
                true => "UI".into(),
            },
            false => "type0 IL2P".into(),
        }
    }
}
