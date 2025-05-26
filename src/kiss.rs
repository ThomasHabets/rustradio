use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagValue};

const KISS_FEND: u8 = 0xC0;
const KISS_FESC: u8 = 0xDB;
const KISS_TFEND: u8 = 0xDC;
const KISS_TFESC: u8 = 0xDD;

/// Escape KISS data stream.
///
/// https://en.wikipedia.org/wiki/KISS_(amateur_radio_protocol)
#[must_use]
fn escape(bytes: &[u8]) -> Vec<u8> {
    // Add 10% capacity to leave room for escaped
    let mut ret = Vec::with_capacity((3 + bytes.len()) * 110 / 100);
    ret.push(KISS_FEND);
    ret.push(0); // TODO: port
    for &b in bytes {
        match b {
            KISS_FEND => ret.extend(vec![KISS_FESC, KISS_TFEND]),
            KISS_FESC => ret.extend(vec![KISS_FESC, KISS_TFESC]),
            b => ret.push(b),
        }
    }
    ret.push(KISS_FEND);
    ret
}

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct KissDecode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
}

impl Block for KissDecode {
    fn work(&mut self) -> Result<BlockRet> {
        let _ = &self.src;
        let _ = &self.dst;
        todo!()
    }
}

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct KissEncode {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
}

impl Block for KissEncode {
    fn work(&mut self) -> Result<BlockRet> {
        loop {
            let Some((x, mut tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let out = escape(&x);
            tags.push(Tag::new(
                0,
                "KISS:input-bytes",
                TagValue::U64(x.len().try_into().unwrap()),
            ));
            tags.push(Tag::new(
                0,
                "KISS:output-bytes",
                TagValue::U64(out.len().try_into().unwrap()),
            ));
            self.dst.push(out, tags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_nocopy_stream;

    #[test]
    fn encode_nothing() -> Result<()> {
        let (_tx, rx) = new_nocopy_stream();
        let (mut b, out) = KissEncode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.pop(), None);
        Ok(())
    }

    #[test]
    fn encode_empty() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(vec![], &[]);
        let (mut b, out) = KissEncode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (o, tags) = out.pop().unwrap();
        assert_eq!(o, &[KISS_FEND, 0, KISS_FEND]);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "KISS:input-bytes", TagValue::U64(0)),
                Tag::new(0, "KISS:output-bytes", TagValue::U64(3)),
            ]
        );
        Ok(())
    }

    #[test]
    fn encode_some_data() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(
            b"fo\xC0o\xDB".to_vec(),
            &[Tag::new(0, "foobar", TagValue::String("baz".to_string()))],
        );
        let (mut b, out) = KissEncode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (o, tags) = out.pop().unwrap();
        let want = &[
            KISS_FEND, 0, b'f', b'o', KISS_FESC, KISS_TFEND, b'o', KISS_FESC, KISS_TFESC, KISS_FEND,
        ];
        assert_eq!(o, want);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "foobar", TagValue::String("baz".to_string())),
                Tag::new(0, "KISS:input-bytes", TagValue::U64(5)),
                Tag::new(
                    0,
                    "KISS:output-bytes",
                    TagValue::U64(want.len().try_into().unwrap())
                ),
            ]
        );
        Ok(())
    }
}
