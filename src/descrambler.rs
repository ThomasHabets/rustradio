/*! LFSR based Descrambler.

AX.25 G3RUH uses mask 0x21 and length 16. Seed doesn't matter, since
by the time the packet arrives the original seed will be shifted out
anyway.
 */
use crate::stream::{Stream, Streamp};

/// LFSR as used by G3RUH.
///
/// Input bit is added to the beginning of the shift register, and the
/// output is taken from the mask.
struct Lfsr {
    mask: u64,
    len: u8,
    shift_reg: u64,
}

impl Lfsr {
    /// Create new LFSR.
    fn new(mask: u64, seed: u64, len: u8) -> Self {
        assert!(len < 64);
        Self {
            mask,
            len,
            shift_reg: seed,
        }
    }
    /// Clock the LFSR.
    fn next(&mut self, i: u8) -> u8 {
        assert!(i <= 1);
        let ret = 1 & (self.shift_reg & self.mask).count_ones() as u8 ^ i;
        self.shift_reg = (self.shift_reg >> 1) | ((i as u64) << self.len);
        ret
    }
}

/// Descrambler uses an LFSR to descramble bits.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, out, sync)]
pub struct Descrambler {
    #[rustradio(in)]
    src: Streamp<u8>,
    #[rustradio(out)]
    dst: Streamp<u8>,
    lfsr: Lfsr,
}
impl Descrambler {
    /// Create new descrambler.
    // TODO: take an lfsr, partly so that we can generate this new()
    pub fn new(src: Streamp<u8>, mask: u64, seed: u64, len: u8) -> Self {
        Self {
            src,
            dst: Stream::newp(),
            lfsr: Lfsr::new(mask, seed, len),
        }
    }

    /// Create a descrambler with G3RUH parameters.
    pub fn new_g3ruh(src: Streamp<u8>) -> Self {
        Self {
            src,
            dst: Stream::newp(),
            lfsr: Lfsr::new(0x21, 0, 16),
        }
    }

    fn process_sync(&mut self, bit: u8) -> u8 {
        self.lfsr.next(bit)
    }
}
