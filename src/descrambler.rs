/*! LFSR based Descrambler.

AX.25 G3RUH uses mask 0x21 and length 16. Seed doesn't matter, since
by the time the packet arrives the original seed will be shifted out
anyway.
 */
use crate::map_block_convert_macro;
use crate::stream::{new_streamp, ReadStream, ReadStreamp, Streamp};

struct Lfsr {
    mask: u64,
    len: u8,
    shift_reg: u64,
}

impl Lfsr {
    fn new(mask: u64, seed: u64, len: u8) -> Self {
        assert!(len < 64);
        Self {
            mask,
            len,
            shift_reg: seed,
        }
    }
    fn next(&mut self, i: u8) -> u8 {
        assert!(i <= 1);
        let ret = 1 & (self.shift_reg & self.mask).count_ones() as u8 ^ i;
        self.shift_reg = (self.shift_reg >> 1) | ((i as u64) << self.len);
        ret
    }
}

/// Descrambler uses an LFSR to descramble bits.
pub struct Descrambler {
    src: ReadStreamp<u8>,
    dst: Streamp<u8>,
    lfsr: Lfsr,
}
impl Descrambler {
    /// Create new descrambler.
    pub fn new(src: ReadStreamp<u8>, mask: u64, seed: u64, len: u8) -> Self {
        Self {
            src,
            dst: new_streamp(),
            lfsr: Lfsr::new(mask, seed, len),
        }
    }

    fn process_one(&mut self, bit: u8) -> u8 {
        self.lfsr.next(bit)
    }
}

map_block_convert_macro![Descrambler, u8];
