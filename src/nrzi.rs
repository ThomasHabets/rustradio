/*! NRZI — Non return to zero

The same effect can be had by doing:

```text
let (a, b) = add_block![g, Tee::new(prev)];
let delay = add_block![g, Delay::new(a, 1)];
let prev = add_block![g, Xor::new(delay, b)];
let prev = add_block![g, XorConst::new(prev, 1u8)];
```
*/
use crate::map_block_convert_macro;
use crate::stream::{new_streamp, Streamp};

/// NRZI decoder.
pub struct NrziDecode {
    last: u8,
    src: Streamp<u8>,
    dst: Streamp<u8>,
}

impl NrziDecode {
    /// Create a new NRZI block.
    pub fn new(src: Streamp<u8>) -> Self {
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
