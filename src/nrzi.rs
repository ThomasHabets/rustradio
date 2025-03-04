//! NRZI — Non return to zero
use crate::stream::{ReadStream, WriteStream};

/// NRZI decoder.
///
/// <https://en.wikipedia.org/wiki/Non-return-to-zero>
///
/// The same effect as NrziDecode can be had by doing:
///
/// ```text
/// let (a, b) = add_block![g, Tee::new(prev)];
/// let delay = add_block![g, Delay::new(a, 1)];
/// let prev = add_block![g, Xor::new(delay, b)];
/// let prev = add_block![g, XorConst::new(prev, 1u8)];
/// ```
///
/// "NRZI" is actually ambiguous as to which is zero and which is
/// one. This code is going with NRZI-S, meaning a toggle is zero, and
/// constant is one, because that's what done by AX.25, both 1200bps Bell
/// 202, and 9600 G3RUH.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct NrziDecode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    #[rustradio(default)]
    last: u8,
}

impl NrziDecode {
    fn process_sync(&mut self, a: u8) -> u8 {
        let tmp = self.last;
        self.last = a;
        1 ^ a ^ tmp
    }
}
