/*! IL2P Deframer

*/
use log::info;

use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{NCWriteStream, ReadStream, Tag};

const HEADER_SIZE: usize = 15 * 8;

/// SYNC_WORD is the pattern of bits (after the clock sync preamble) that
/// indicate the start of an IL2P frame.
///
/// Another word for these bits is 0xF15E48.
pub const SYNC_WORD: [u8; 24] = [
    1, 1, 1, 1, 0, 0, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0,
];

/// Protocol identifier, a concept inherited from AX.25, but IL2P uses
/// different numbers for them, and bakes in the frame type with the PID.
pub struct Pids {}
impl Pids {
    /// AX.25 supervisor frames. E.g. RR, SREJ, …
    ///
    /// These frames don't have a PID field in AX.25.
    pub const AX25_SUPERVISOR: u8 = 0;

    /// AX.25 unnumbered frames. E.g. SABM, DM, UA, …
    ///
    /// These frames don't have a PID field in AX.25.
    pub const AX25_UNNUMBERED: u8 = 1;

    /// AX.25 layer3.
    ///
    /// yy10yyyy or yy01yyyy in AX.25.
    pub const AX25_LAYER3: u8 = 2;

    /// ISO 8208/CCITT X.25 PLP.
    ///
    /// Whatever that is. 1 in AX.25.
    pub const ISO_8208_CCIT_X25_PLP: u8 = 3;

    /// Compressed TCP/IP
    ///
    /// 6 in AX.25.
    pub const COMPRESSED_TCPIP: u8 = 4;

    /// Uncompressed TCP/IP.
    ///
    /// 7 in AX.25.
    pub const UNCOMPRESSED_TCPIP: u8 = 5;

    /// Segmentation fragment.
    ///
    /// 8 in AX.25.
    pub const SEGMENTATION_FRAGMENT: u8 = 6;

    /// Reserved for future use.
    pub const FUTURE7: u8 = 7;

    /// Reserved for future use.
    pub const FUTURE8: u8 = 8;

    /// Reserved for future use.
    pub const FUTURE9: u8 = 9;

    /// Reserved for future use.
    pub const FUTURE10: u8 = 10;

    /// ARPA Internet protocol.
    ///
    /// 0xCC in AX.25.
    pub const ARPA_IP: u8 = 11;

    /// ARPA Address Resolution.
    ///
    /// 0xCD in AX.25.
    pub const ARPA_ADDRESS_RESOLUTION: u8 = 12;

    /// FlexNet
    ///
    /// 0xCE in AX.25.
    pub const FLEX_NET: u8 = 13;

    /// TheNET
    ///
    /// 0xCF in AX.25.
    pub const THE_NET: u8 = 14;

    /// No L3.
    ///
    /// Used by e.g. APRS. But because a type 1 header doesn't have
    /// room for repeaters, this constant will normally not be used
    /// for APRS over IL2P.
    ///
    /// 0xF0 in AX.25.
    pub const NO_L3: u8 = 15;
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
    assert![bits.len().is_multiple_of(8)];
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

#[derive(Default)]
enum State {
    #[default]
    Unsynced,
    Header(Vec<u8>),
    //Data(Vec<u8>, usize),
}

/// IL2P deframer block
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Il2pDeframer {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
    #[rustradio(default)]
    decoded: usize,
    #[rustradio(default)]
    state: State,
}

impl Drop for Il2pDeframer {
    fn drop(&mut self) {
        info!("IL2P Deframer: Decoded {}", self.decoded);
    }
}

impl Block for Il2pDeframer {
    fn work(&mut self) -> Result<BlockRet> {
        let (input, tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
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

            // TODO: push something useful.
            self.dst.push(Vec::new(), &[]);
        } else if let Some(Err(e)) = header {
            info!("Failed to parse header: {e}");
        }
        Ok(BlockRet::Again)
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
            pid: ((data[1] & 0x40) >> 3)
                | ((data[2] & 0x40) >> 4)
                | ((data[3] & 0x40) >> 5)
                | ((data[4] & 0x40) >> 6),
            control: (data[5] & 0x40)
                | ((data[6] & 0x40) >> 1)
                | ((data[7] & 0x40) >> 2)
                | ((data[8] & 0x40) >> 3)
                | ((data[9] & 0x40) >> 4)
                | ((data[10] & 0x40) >> 5)
                | ((data[11] & 0x40) >> 6),
            payload_size: ((data[2] as u16 & 0x80) << 2)
                | ((data[3] as u16 & 0x80) << 1)
                | (data[4] as u16 & 0x80)
                | ((data[5] as u16 & 0x80) >> 1)
                | ((data[6] as u16 & 0x80) >> 2)
                | ((data[7] as u16 & 0x80) >> 3)
                | ((data[8] as u16 & 0x80) >> 4)
                | ((data[9] as u16 & 0x80) >> 5)
                | ((data[10] as u16 & 0x80) >> 6)
                | ((data[11] as u16 & 0x80) >> 7),
        })
    }
    fn describe(&self) -> String {
        match self.hdrtype1 {
            true => match self.ui {
                false => match self.pid {
                    Pids::AX25_UNNUMBERED => match (self.control >> 2) & 0xF {
                        0x0 => "invalid 0x00",
                        0x1 => "SABM",
                        0x2 => "invalid 0x02",
                        0x3 => "DISC",
                        0x4 => "DM",
                        0x5 => "invalid 0x05",
                        0x6 => "UA",
                        0x7 => "invalid 0x07",
                        0x8 => "FRMR",
                        0x9 => "unvalid 0x09",
                        0xA => "UI unnumbered response",
                        0xB => "UI unnumbered command",
                        0xC => "XID response",
                        0xD => "XID command",
                        0xE => "TEST response",
                        0xF => "TEST command",
                        16.. => "Can't happen",
                    },
                    _ => "other PID",
                },
                true => "UI",
            },
            false => "type0 IL2P",
        }
        .into()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::stream::ReadStream;

    use crate::blocks::CorrelateAccessCodeTag;

    use std::fs::File;
    use std::io::Read;
    use std::path::Path;

    fn read_binary_file_as_u8<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<u8>> {
        let mut file = File::open(path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    #[test]
    fn test_header_decode() -> Result<()> {
        let src = ReadStream::from_slice(&read_binary_file_as_u8("testdata/il2p.bits")?);
        let (mut cac, cac_out) = CorrelateAccessCodeTag::new(src, SYNC_WORD.to_vec(), "sync", 0);
        let (mut deframer, o) = Il2pDeframer::new(cac_out);
        cac.work()?;
        deframer.work()?;
        deframer.work()?;
        let _ = o.pop().expect("expected to get a parsed packet");
        // TODO: confirm parsing.
        if let Some(res) = o.pop() {
            panic!("got a second packet: {res:?}");
        }
        Ok(())
    }
}
