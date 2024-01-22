/*! Correlate Access Code blocks.

For now an initial yes/no bit block. Future work should add tagging.
*/
use crate::map_block_convert_macro;
use crate::map_block_convert_tag_macro;
use crate::stream::{new_streamp, Streamp, Tag, TagValue};

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
            dst: new_streamp(),
            slide: vec![0; code.len()],
            code,
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

/// CorrelateAccessCode outputs 1 if CAC matches.
pub struct CorrelateAccessCodeTag {
    code: Vec<u8>,
    src: Streamp<u8>,
    dst: Streamp<u8>,
    slide: Vec<u8>,
    allowed_diffs: usize,
    tag: String,
}

impl CorrelateAccessCodeTag {
    /// Create new correlate access block.
    pub fn new(src: Streamp<u8>, code: Vec<u8>, tag: String, allowed_diffs: usize) -> Self {
        Self {
            src,
            tag,
            dst: new_streamp(),
            slide: vec![0; code.len()],
            code,
            allowed_diffs,
        }
    }
    fn process_one(&mut self, a: u8, tags: &[Tag]) -> (u8, Vec<Tag>) {
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
        let mut tags = tags.to_vec();
        if diffs <= self.allowed_diffs {
            tags.push(Tag::new(
                0,
                self.tag.clone(),
                TagValue::U64(
                    diffs
                        .try_into()
                        .expect("can't happen: usize doesn't fit in u64"),
                ),
            ));
        }
        (a, tags)
    }
}
map_block_convert_tag_macro![CorrelateAccessCodeTag, u8];
