/*! LFSR based Descrambler.

AX.25 G3RUH uses mask 0x21 and length 16. Seed doesn't matter, since
by the time the packet arrives the original seed will be shifted out
anyway.
 */
use crate::stream::{ReadStream, WriteStream};

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
    /// Create new G3RUH LFSR.
    fn g3ruh() -> Self {
        Self::new(0x21, 0, 16)
    }
    /// Clock the LFSR.
    fn next_descramble(&mut self, i: u8) -> u8 {
        assert!(i <= 1);
        let ret = 1 & (self.shift_reg & self.mask).count_ones() as u8 ^ i;
        self.shift_reg = (self.shift_reg >> 1) | ((i as u64) << self.len);
        ret
    }
    /// Clock the LFSR.
    fn next_scramble(&mut self, i: u8) -> u8 {
        assert!(i <= 1);
        let ret = (self.shift_reg & 1) as u8;
        let tmp = 1 & (self.shift_reg & self.mask).count_ones() as u8 ^ i;
        self.shift_reg = (self.shift_reg >> 1) | ((tmp as u64) << self.len);
        ret
    }
}

/// Descrambler uses an LFSR to descramble bits.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct Descrambler {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    lfsr: Lfsr,
}
impl Descrambler {
    /// Create new descrambler.
    // TODO: take an lfsr, partly so that we can generate this new()
    pub fn new(src: ReadStream<u8>, mask: u64, seed: u64, len: u8) -> (Self, ReadStream<u8>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                lfsr: Lfsr::new(mask, seed, len),
            },
            dr,
        )
    }

    /// Create a descrambler with G3RUH parameters.
    pub fn g3ruh(src: ReadStream<u8>) -> (Self, ReadStream<u8>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                lfsr: Lfsr::g3ruh(),
            },
            dr,
        )
    }

    fn process_sync(&mut self, bit: u8) -> u8 {
        self.lfsr.next_descramble(bit)
    }
}

/// Scrambler uses an LFSR to scramble bits.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct Scrambler {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    lfsr: Lfsr,
}
impl Scrambler {
    /// Create a descrambler with G3RUH parameters.
    pub fn g3ruh(src: ReadStream<u8>) -> (Self, ReadStream<u8>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                lfsr: Lfsr::g3ruh(),
            },
            dr,
        )
    }

    fn process_sync(&mut self, bit: u8) -> u8 {
        self.lfsr.next_scramble(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::blocks::{NrziDecode, NrziEncode, VectorSource};

    #[test]
    fn known_good_test1() {
        let len = 16;
        let input = vec![1, 1, 1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 1, 0];

        let scrambled = {
            let mut l1 = Lfsr::g3ruh();
            let scrambled: Vec<_> = input
                .iter()
                .copied()
                .chain(vec![0u8; len + 1])
                .map(|s| l1.next_scramble(s))
                .skip(17)
                .collect();
            scrambled
        };
        assert_ne!(scrambled, input);
        assert_eq!(
            scrambled,
            vec![1, 1, 1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 0, 0, 1,]
        );

        let mut l2 = Lfsr::g3ruh();
        let descrambled: Vec<_> = scrambled
            .iter()
            .copied()
            .chain(vec![0u8; len])
            .map(|s| l2.next_descramble(s))
            .take(input.len())
            .collect();
        assert_eq!(descrambled, input);
    }

    #[test]
    fn known_good_ones() {
        let len = 16;
        let input = vec![1u8; 24];

        let scrambled = {
            let mut l1 = Lfsr::g3ruh();
            let scrambled: Vec<_> = input
                .iter()
                .copied()
                .chain(vec![0u8; len + 1])
                .map(|s| l1.next_scramble(s))
                .skip(17)
                .collect();
            scrambled
        };
        assert_ne!(scrambled, input);
        assert_eq!(
            scrambled,
            vec![
                1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1,
            ]
        );

        let mut l2 = Lfsr::g3ruh();
        let descrambled: Vec<_> = scrambled
            .iter()
            .copied()
            .chain(vec![0u8; len])
            .map(|s| l2.next_descramble(s))
            .take(input.len())
            .collect();
        assert_eq!(descrambled, input);
    }

    #[test]
    fn long_random() {
        use rand::Rng;
        let mut rng = rand::rng();
        let len = 2000;
        let input: Vec<_> = (0..len).map(|_| rng.random_range(0..=1)).collect();

        let scrambled = {
            let mut l1 = Lfsr::g3ruh();
            let scrambled: Vec<_> = input
                .iter()
                .copied()
                .chain(vec![0u8; len + 1])
                .map(|s| l1.next_scramble(s))
                .skip(17)
                .collect();
            scrambled
        };

        let mut l2 = Lfsr::g3ruh();
        let descrambled: Vec<_> = scrambled
            .iter()
            .copied()
            .chain(vec![0u8; len])
            .map(|s| l2.next_descramble(s))
            .take(input.len())
            .collect();
        assert_eq!(descrambled, input);
    }

    #[test]
    fn long_random_nrzi_g3ruh() {
        use rand::Rng;
        let mut rng = rand::rng();
        let len = 2000;
        let input: Vec<_> = (0..len).map(|_| rng.random_range(0..=1)).collect();
        let pad: Vec<_> = (0..17).map(|_| rng.random_range(0..=1)).collect();

        let (mut b, prev) = VectorSource::new(input.iter().copied().chain(pad).collect());
        b.work().unwrap();
        let (mut b, prev) = NrziEncode::new(prev);
        b.work().unwrap();
        let (mut b, prev) = Scrambler::g3ruh(prev);
        b.work().unwrap();
        let (mut b, prev) = Descrambler::g3ruh(prev);
        b.work().unwrap();
        let (mut b, prev) = NrziDecode::new(prev);
        b.work().unwrap();

        let (out, _) = prev.read_buf().unwrap();
        let descrambled: Vec<_> = out.iter().copied().skip(17).collect();
        assert_eq!(descrambled, input);
    }
}
