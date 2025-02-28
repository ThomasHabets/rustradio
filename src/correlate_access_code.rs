/*! Correlate Access Code blocks.

For now an initial yes/no bit block. Future work should add tagging.
*/
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};

/// CorrelateAccessCode outputs 1 if CAC matches.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct CorrelateAccessCode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,

    code: Vec<u8>,
    slide: Vec<u8>,
    allowed_diffs: usize,
}

impl CorrelateAccessCode {
    /// Create new correlate access block.
    pub fn new(src: ReadStream<u8>, code: Vec<u8>, allowed_diffs: usize) -> (Self, ReadStream<u8>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                slide: vec![0; code.len()],
                code,
                allowed_diffs,
            },
            dr,
        )
    }
    fn process_sync(&mut self, a: u8) -> u8 {
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
        if diffs <= self.allowed_diffs { 1 } else { 0 }
    }
}

/// CorrelateAccessCode outputs 1 if CAC matches.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync_tag)]
pub struct CorrelateAccessCodeTag {
    code: Vec<u8>,
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    slide: Vec<u8>,
    allowed_diffs: usize,
    tag: String,
}

impl CorrelateAccessCodeTag {
    /// Create new correlate access block.
    pub fn new(
        src: ReadStream<u8>,
        code: Vec<u8>,
        tag: String,
        allowed_diffs: usize,
    ) -> (Self, ReadStream<u8>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                tag,
                dst,
                slide: vec![0; code.len()],
                code,
                allowed_diffs,
            },
            dr,
        )
    }
    fn process_sync_tags(&mut self, a: u8, tags: &[Tag]) -> (u8, Vec<Tag>) {
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
