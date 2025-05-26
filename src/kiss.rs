use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, Tag, TagValue};
use crate::{Error, Result};

const KISS_FEND: u8 = 0xC0;
const KISS_FESC: u8 = 0xDB;
const KISS_TFEND: u8 = 0xDC;
const KISS_TFESC: u8 = 0xDD;

/// Escape KISS data stream.
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

/// Unescape KISS data stream.
fn unescape(data: &[u8]) -> Result<Vec<u8>> {
    let mut unescaped = Vec::with_capacity(data.len());
    let mut is_escaped = false;
    for &byte in data {
        if is_escaped {
            unescaped.push(match byte {
                KISS_TFESC => KISS_FESC,
                KISS_TFEND => KISS_FEND,
                other => {
                    return Err(Error::msg(format!(
                        "KissDecode: invalid escape byte {other:02x}"
                    )));
                }
            });
            is_escaped = false;
        } else if byte == KISS_FESC {
            // Next byte is escaped, so set the flag
            is_escaped = true;
        } else if byte == KISS_FEND {
            return Err(Error::msg("KissDecode: FEND in the middle of a packet"));
        } else {
            // Normal byte, just push it to the output
            unescaped.push(byte);
        }
    }
    if is_escaped {
        Err(Error::msg("KissDecode: ended on an escape"))
    } else {
        Ok(unescaped)
    }
}

/// Decode KISS frame.
///
/// <https://en.wikipedia.org/wiki/KISS_(amateur_radio_protocol)>
///
/// TODO: Tag with other KISS stuff like channel.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct KissDecode {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
}

impl Block for KissDecode {
    fn work(&mut self) -> Result<BlockRet> {
        loop {
            let Some((x, mut tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let out = match unescape(&x) {
                Ok(o) => o,
                Err(e) => {
                    log::debug!("Bad KISS packet: {e}");
                    continue;
                }
            };
            tags.push(Tag::new(
                0,
                "KissDecode:input-bytes",
                TagValue::U64(x.len().try_into().unwrap()),
            ));
            tags.push(Tag::new(
                0,
                "KissDecode:output-bytes",
                TagValue::U64(out.len().try_into().unwrap()),
            ));
            self.dst.push(out, tags);
        }
    }
}

/// Kiss encode.
///
/// Takes bytes and creates a KISS frame.
///
/// <https://en.wikipedia.org/wiki/KISS_(amateur_radio_protocol)>
///
/// TODO: Take channel from a tag.
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
                "KissEncode:input-bytes",
                TagValue::U64(x.len().try_into().unwrap()),
            ));
            tags.push(Tag::new(
                0,
                "KissEncode:output-bytes",
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
                Tag::new(0, "KissEncode:input-bytes", TagValue::U64(0)),
                Tag::new(0, "KissEncode:output-bytes", TagValue::U64(3)),
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
                Tag::new(0, "KissEncode:input-bytes", TagValue::U64(5)),
                Tag::new(
                    0,
                    "KissEncode:output-bytes",
                    TagValue::U64(want.len().try_into().unwrap())
                ),
            ]
        );
        Ok(())
    }
}
