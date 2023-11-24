/*! NRZI — Non return to zero

<https://en.wikipedia.org/wiki/Non-return-to-zero>

The same effect as NrziDecode can be had by doing:

```text
let (a, b) = add_block![g, Tee::new(prev)];
let delay = add_block![g, Delay::new(a, 1)];
let prev = add_block![g, Xor::new(delay, b)];
let prev = add_block![g, XorConst::new(prev, 1u8)];
```

"NRZI" is actually ambiguous as to which is zero and which is
one. This code is going with NRZI-S, meaning a toggle is zero, and
constant is one, because that's what done by AX.25, both 1200bps Bell
202, and 9600 G3RUH.
*/
use crate::map_block_convert_macro;
use crate::stream::{new_streamp, Streamp, ReadStreamp};

/// NRZI decoder.
pub struct NrziDecode {
    last: u8,
    src: ReadStreamp<u8>,
    dst: Streamp<u8>,
}

impl NrziDecode {
    /// Create a new NRZI block.
    pub fn new(src: ReadStreamp<u8>) -> Self {
        Self {
            src,
            dst: new_streamp(),
            last: 0,
        }
    }

    fn process_one(&mut self, a: u8) -> u8 {
        let tmp = self.last;
        self.last = a;
        1 ^ a ^ tmp
    }
}
map_block_convert_macro![NrziDecode, u8];
