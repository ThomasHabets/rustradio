/*! Correlate Access Code blocks.

For now an initial yes/no bit block. Future work should add tagging.
*/
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};

/// `CorrelateAccessCode` outputs 1 if CAC matches.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct CorrelateAccessCode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,

    code: Box<[u8]>,
    slide: Vec<u8>,
    allowed_diffs: usize,
}

impl CorrelateAccessCode {
    /// Create a correlator with an owned access code, returning an error if empty.
    pub fn new(
        src: ReadStream<u8>,
        code: impl Into<Box<[u8]>>,
        allowed_diffs: usize,
    ) -> crate::Result<(Self, ReadStream<u8>)> {
        let code = code.into();
        if code.is_empty() {
            return Err(crate::Error::msg("access code must be nonempty"));
        }
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((
            Self {
                src,
                dst,
                slide: Vec::with_capacity(code.len()),
                code,
                allowed_diffs,
            },
            dr,
        ))
    }
    fn process_sync(&mut self, a: u8) -> u8 {
        self.slide.push(a);

        if self.slide.len() > self.code.len() {
            self.slide.remove(0);
        }
        if self.slide.len() < self.code.len() {
            return 0;
        }
        let diffs = self
            .slide
            .iter()
            .zip(&self.code)
            .filter(|(a, b)| a != b)
            .count();
        u8::from(diffs <= self.allowed_diffs)
    }
}

/// `CorrelateAccessCode` outputs 1 if CAC matches.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync_tag)]
pub struct CorrelateAccessCodeTag {
    code: Box<[u8]>,
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    slide: Vec<u8>,
    allowed_diffs: usize,
    tag: String,
}

impl CorrelateAccessCodeTag {
    /// Create a tagged correlator with an owned access code, returning an error if empty.
    pub fn new<S: Into<String>>(
        src: ReadStream<u8>,
        code: impl Into<Box<[u8]>>,
        tag: S,
        allowed_diffs: usize,
    ) -> crate::Result<(Self, ReadStream<u8>)> {
        let code = code.into();
        if code.is_empty() {
            return Err(crate::Error::msg("access code must be nonempty"));
        }
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((
            Self {
                src,
                tag: tag.into(),
                dst,
                slide: Vec::with_capacity(code.len()),
                code,
                allowed_diffs,
            },
            dr,
        ))
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
        if self.slide.len() == self.code.len() && diffs <= self.allowed_diffs {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::block::Block;

    #[test]
    fn rejects_empty_code() {
        assert!(CorrelateAccessCode::new(ReadStream::from_slice(&[]), vec![], 0).is_err());
    }

    #[test]
    fn tagged_rejects_empty_code() {
        assert!(
            CorrelateAccessCodeTag::new(ReadStream::from_slice(&[]), vec![], "sync", 0).is_err()
        );
    }

    #[test]
    fn waits_for_full_code_before_match() -> Result<()> {
        let src = ReadStream::from_slice(&[1]);
        let (mut cac, out) = CorrelateAccessCode::new(src, [0, 1], 0)?;

        cac.work()?;
        let (buf, _) = out.read_buf()?;

        assert_eq!(buf.slice(), &[0]);
        Ok(())
    }

    #[test]
    fn tagged_waits_for_full_code_before_match() -> Result<()> {
        let src = ReadStream::from_slice(&[1]);
        let (mut cac, out) = CorrelateAccessCodeTag::new(src, &[0, 1][..], "sync", 0)?;

        cac.work()?;
        let (_, tags) = out.read_buf()?;

        assert_eq!(tags, &[]);
        Ok(())
    }
}
