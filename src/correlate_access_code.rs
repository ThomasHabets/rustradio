/*! Correlate Access Code blocks.

For now an initial yes/no bit block. Future work should add tagging.
*/
use crate::map_block_convert_macro;
use crate::stream::{new_streamp, Streamp};

/// CorrelateAccessCode outputs 1 if CAC matches.
pub struct CorrelateAccessCode {
    code: Vec<u8>,
    src: Streamp<u8>,
    dst: Streamp<u8>,
    slide: Vec<u8>,
}

impl CorrelateAccessCode {
    /// Create new correlate access block.
    pub fn new(src: Streamp<u8>, code: Vec<u8>) -> Self {
        Self {
            src,
            code,
            dst: new_streamp(),
            slide: Vec::new(),
        }
    }
    fn process_one(&mut self, a: u8) -> u8 {
        self.slide.push(a);
        if self.slide.len() > self.code.len() {
            self.slide.remove(0);
        }
        if self.slide == self.code {
            1
        } else {
            0
        }
    }
}
map_block_convert_macro![CorrelateAccessCode, u8];
