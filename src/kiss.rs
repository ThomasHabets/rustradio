use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagValue};
use crate::{Error, Result};
use log::debug;

const MAX_LEN: usize = 10_000;
const KISS_FEND: u8 = 0xC0;
const KISS_FESC: u8 = 0xDB;
const KISS_TFEND: u8 = 0xDC;
const KISS_TFESC: u8 = 0xDD;
const ENCODE_PORT_TAG: &str = "KissEncode:port";

fn strip_fend(data: &[u8]) -> &[u8] {
    let start = data
        .iter()
        .position(|&b| b != KISS_FEND)
        .unwrap_or(data.len());
    if start == data.len() {
        return &[];
    }
    let end = data
        .iter()
        .rposition(|&b| b != KISS_FEND)
        .map_or(0, |i| i + 1);
    &data[start..end]
}

/// Escape KISS data stream.
#[must_use]
fn escape(bytes: &[u8], port: u8) -> Vec<u8> {
    // Add 10% capacity to leave room for escaped
    let mut ret = Vec::with_capacity((3 + bytes.len()) * 110 / 100);
    ret.push(KISS_FEND);
    for b in std::iter::once(port << 4).chain(bytes.iter().copied()) {
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
/// This means trimming away `FEND` from both sides, and unescaping `HDLC`
/// bytes.
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
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            if self.dst.remaining() == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let Some((x, mut tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let x = strip_fend(&x);
            if x.is_empty() {
                continue;
            }
            let mut out = match unescape(x) {
                Ok(o) => o,
                Err(e) => {
                    log::debug!("Bad KISS packet: {e}");
                    continue;
                }
            };
            let port = out.remove(0);
            if port & 0xF != 0 {
                debug!("KissDecode: non-data packet: {port:02x} {out:02x?}");
                continue;
            }
            let port = (port >> 4) & 0xf;
            // Count encoded payload bytes without the possibly escaped command.
            let command_bytes = if x[0] == KISS_FESC { 2 } else { 1 };
            tags.extend(vec![
                Tag::new(0, "KissDecode:port", TagValue::U64(port.into())),
                Tag::new(
                    0,
                    "KissDecode:input-bytes",
                    TagValue::U64((x.len() - command_bytes).try_into().unwrap()),
                ),
                Tag::new(
                    0,
                    "KissDecode:output-bytes",
                    TagValue::U64(out.len().try_into().unwrap()),
                ),
            ]);
            self.dst.push(out, tags);
        }
    }
}

#[derive(Default)]
enum FrameState {
    #[default]
    Unsynced,
    Synced(Vec<u8>),
}

impl FrameState {}

/// Kiss frame.
///
/// Take stream of bytes and output the still KISS encoded frames.
/// Should probably be followed by `KissDecode`.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct KissFrame {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,

    #[rustradio(default)]
    state: FrameState,
}

impl Block for KissFrame {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            if self.dst.remaining() == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let old_state = std::mem::replace(&mut self.state, FrameState::Unsynced);
            self.state = match old_state {
                FrameState::Unsynced => {
                    let (i, _tags) = self.src.read_buf()?;
                    if i.is_empty() {
                        return Ok(BlockRet::WaitForStream(&self.src, 1));
                    }
                    let mut n = 0;
                    let mut synced = false;
                    for sample in i.slice().iter().copied() {
                        n += 1;
                        if sample == KISS_FEND {
                            synced = true;
                            break;
                        }
                    }
                    i.consume(n);
                    if synced {
                        FrameState::Synced(vec![])
                    } else {
                        FrameState::Unsynced
                    }
                }
                FrameState::Synced(mut v) => {
                    let (i, _tags) = self.src.read_buf()?;
                    if i.is_empty() {
                        return Ok(BlockRet::WaitForStream(&self.src, 1));
                    }
                    let mut n = 0;
                    let mut done = false;
                    for sample in i.slice().iter().copied() {
                        n += 1;
                        if sample == KISS_FEND {
                            if v.is_empty() {
                                continue;
                            }
                            done = true;
                            break;
                        }
                        v.push(sample);
                        if v.len() > MAX_LEN {
                            break;
                        }
                    }
                    i.consume(n);
                    if v.len() > MAX_LEN {
                        FrameState::Unsynced
                    } else if done {
                        // TODO: add tags.
                        self.dst.push(v, &[]);
                        FrameState::Synced(vec![])
                    } else {
                        FrameState::Synced(v)
                    }
                }
            };
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
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            if self.dst.remaining() == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let Some((x, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            let port = match tags
                .iter()
                .find(|t| t.key() == ENCODE_PORT_TAG)
                .map_or(&TagValue::U64(0), super::stream::Tag::val)
            {
                TagValue::U64(port) if *port < 0x10 => *port & 0xf,
                other => {
                    debug!("KissEncode: invalid port tag value: {other:?}");
                    0
                }
            };
            let out = escape(&x, port.try_into().unwrap());
            let tags: Vec<_> = tags
                .into_iter()
                .filter(|t| t.key() != ENCODE_PORT_TAG)
                .chain(vec![
                    Tag::new(
                        0,
                        "KissEncode:input-bytes",
                        TagValue::U64(x.len().try_into().unwrap()),
                    ),
                    Tag::new(
                        0,
                        "KissEncode:output-bytes",
                        TagValue::U64(out.len().try_into().unwrap()),
                    ),
                ])
                .collect();
            self.dst.push(out, tags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::stream::new_nocopy_stream;

    #[test]
    fn decode_nothing() -> Result<()> {
        let (_tx, rx) = new_nocopy_stream();
        let (mut b, out) = KissDecode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.pop(), None);
        Ok(())
    }

    #[test]
    fn decode_empty() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(vec![], &[]);
        let (mut b, out) = KissDecode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        assert_eq!(out.pop(), None);
        Ok(())
    }

    #[test]
    fn decode_some_data() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(
            b"\xC0\x30foo\xDB\xDCA\xDB\xDD".to_vec(),
            &[Tag::new(0, "foobar", TagValue::String("baz".to_string()))],
        );
        let (mut b, out) = KissDecode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (o, tags) = out.pop().unwrap();
        let want = b"foo\xC0A\xDB".to_vec();
        assert_eq!(o, want);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "foobar", TagValue::String("baz".to_string())),
                Tag::new(0, "KissDecode:port", TagValue::U64(3)),
                Tag::new(0, "KissDecode:input-bytes", TagValue::U64(8)),
                Tag::new(
                    0,
                    "KissDecode:output-bytes",
                    TagValue::U64(want.len().try_into().unwrap())
                ),
            ]
        );
        Ok(())
    }

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
            &[
                Tag::new(0, "foobar", TagValue::String("baz".to_string())),
                Tag::new(0, "KissEncode:port", TagValue::U64(1)),
            ],
        );
        let (mut b, out) = KissEncode::new(rx);
        assert!(matches![b.work()?, BlockRet::WaitForStream(_, 1)]);
        let (o, tags) = out.pop().unwrap();
        let want = &[
            KISS_FEND, 0x10, b'f', b'o', KISS_FESC, KISS_TFEND, b'o', KISS_FESC, KISS_TFESC,
            KISS_FEND,
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

    #[test]
    fn encode_escapes_port_12_command() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(
            vec![1u8, KISS_FEND, KISS_FESC],
            &[Tag::new(0, ENCODE_PORT_TAG, TagValue::U64(12))],
        );
        let (mut encode, output) = KissEncode::new(rx);
        encode.work()?;
        assert_eq!(
            output.pop().unwrap().0,
            [
                KISS_FEND, KISS_FESC, KISS_TFEND, 1, KISS_FESC, KISS_TFEND, KISS_FESC, KISS_TFESC,
                KISS_FEND,
            ]
        );
        Ok(())
    }

    #[test]
    fn decode_unescapes_port_12_command() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        tx.push(
            vec![
                KISS_FEND, KISS_FESC, KISS_TFEND, 1, KISS_FESC, KISS_TFESC, KISS_FEND,
            ],
            &[],
        );
        let (mut decode, output) = KissDecode::new(rx);
        decode.work()?;
        let (data, tags) = output.pop().expect("port 12 data frame");
        assert_eq!(data, [1, KISS_FESC]);
        assert_eq!(
            tags,
            [
                Tag::new(0, "KissDecode:port", TagValue::U64(12)),
                Tag::new(0, "KissDecode:input-bytes", TagValue::U64(3)),
                Tag::new(0, "KissDecode:output-bytes", TagValue::U64(2)),
            ]
        );
        Ok(())
    }

    #[test]
    fn all_ports_roundtrip_through_framing() -> Result<()> {
        for port in 0..16 {
            for payload in [vec![], vec![0u8, KISS_FEND, KISS_FESC, 1]] {
                let (tx, rx) = new_nocopy_stream();
                tx.push(
                    payload.clone(),
                    &[Tag::new(0, ENCODE_PORT_TAG, TagValue::U64(port))],
                );
                let (mut encode, encoded) = KissEncode::new(rx);
                encode.work()?;
                let wire = encoded.pop().unwrap().0;
                let (mut frame, framed) = KissFrame::new(ReadStream::from_slice(&wire));
                frame.work()?;
                let (mut decode, output) = KissDecode::new(framed);
                decode.work()?;
                let (data, tags) = output.pop().expect("data frame");
                assert_eq!(data, payload, "port {port}");
                assert_eq!(tags[0], Tag::new(0, "KissDecode:port", TagValue::U64(port)));
                assert!(output.pop().is_none());
            }
        }
        Ok(())
    }

    #[test]
    fn find_packet() -> Result<()> {
        let (mut b, prev) = VectorSource::new(vec![0u8, KISS_FEND, 0, 1, 2, 3, KISS_FEND])?;
        b.work()?;
        let (mut b, prev) = KissFrame::new(prev);
        b.work()?;
        let (o, _) = prev.pop().unwrap();
        assert_eq!(o, &[0, 1, 2, 3]);
        Ok(())
    }

    #[test]
    fn frame_accepts_exact_max_len() -> Result<()> {
        let mut data = vec![KISS_FEND];
        data.extend(vec![1; MAX_LEN]);
        data.push(KISS_FEND);
        let (mut src, prev) = VectorSource::new(data)?;
        src.work()?;
        let (mut b, prev) = KissFrame::new(prev);
        b.work()?;
        let (o, _) = prev.pop().unwrap();
        assert_eq!(o.len(), MAX_LEN);
        assert!(prev.pop().is_none());
        Ok(())
    }

    #[test]
    fn frame_rejects_over_max_len() -> Result<()> {
        let mut data = vec![KISS_FEND];
        data.extend(vec![1; MAX_LEN + 1]);
        data.push(KISS_FEND);
        let (mut src, prev) = VectorSource::new(data)?;
        src.work()?;
        let (mut b, prev) = KissFrame::new(prev);
        b.work()?;
        assert!(prev.pop().is_none());
        Ok(())
    }
}
