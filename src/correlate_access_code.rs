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
    allowed_diffs: usize,
}

impl CorrelateAccessCode {
    /// Create new correlate access block.
    pub fn new(src: Streamp<u8>, code: Vec<u8>, allowed_diffs: usize) -> Self {
        Self {
            src,
            code,
            dst: new_streamp(),
            slide: Vec::new(),
            allowed_diffs,
        }
    }
    fn process_one(&mut self, a: u8) -> u8 {
        self.slide.push(a);

        if self.slide.len() > self.code.len() {
            self.slide.remove(0);
        }
        let diffs = self
            .slide
            .iter()
            .zip(&self.code)
            .filter(|(a, b)| a != b)
            .count();
        if diffs <= self.allowed_diffs {
            1
        } else {
            0
        }
    }
}
map_block_convert_macro![CorrelateAccessCode, u8];
