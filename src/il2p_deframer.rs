/*! IL2P Deframer

*/
use log::info;

use crate::block::{Block, BlockRet};
use crate::stream::{Streamp, Tag};
use crate::{Error, Result};

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
    //Header(Vec<u8>),
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
        let header_size = 15 * 8;
        let tags: Vec<Tag> = tags.into_iter().filter(|t| t.key() == "sync").collect();
        let (mut partial, size) = match &self.state {
            State::Unsynced => {
                let n = if tags.is_empty() {
                    input.len()
                } else {
                    tags[0].pos()
                };
                if n > 0 {
                    input.consume(n);
                    return Ok(BlockRet::Ok);
                }
                (Vec::new(), header_size)
            }
            //State::Header(partial) => (partial.to_vec(), header_size),
            //State::Data(partial, size) => (partial.to_vec(), *size),
        };
        let mut n = 0;
        for bit in input.iter() {
            if partial.len() == size + 1 {
                break;
            }
            partial.push(*bit);
            n += 1;
        }
        if partial.len() == size + 1 {
            let partial = &partial[1..];
            info!("header with {} bits: {:?}", n, partial);

            let partial2 = decode(partial);
            info!("unscrambled with bytes: {:?}", partial2);

            let header = bits_to_bytes(&partial2);
            info!("header with bytes: {:?}", &header[6..12]);
            info!("destination callsign: {:?}", decode_callsign(&header[0..6]));
            info!("source callsign: {:?}", decode_callsign(&header[6..12]));

            self.state = State::Unsynced;
        }
        input.consume(n);
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
