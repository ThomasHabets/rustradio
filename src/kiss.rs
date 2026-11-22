use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, TagValue, WriteStream};
use crate::{Error, Result};
use log::debug;

const MAX_LEN: usize = 10_000;
const KISS_FEND: u8 = 0xC0;
const KISS_FESC: u8 = 0xDB;
const KISS_TFEND: u8 = 0xDC;
const KISS_TFESC: u8 = 0xDD;
const KISS_DATA: u8 = 0x00;
const KISS_SET_HARDWARE: u8 = 0x06;
const MESHCORE_CHANNEL_BUSY: u8 = 0x8e;
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

/// Escape and frame a complete KISS type byte and its payload.
#[must_use]
fn escape(bytes: &[u8], command: u8) -> Vec<u8> {
    // Add 10% capacity to leave room for escaped
    let mut ret = Vec::with_capacity((3 + bytes.len()) * 110 / 100);
    ret.push(KISS_FEND);
    for b in std::iter::once(command).chain(bytes.iter().copied()) {
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

    // Optional edge-triggered output indicating channel busy.
    #[rustradio(out, default)]
    busy: Option<WriteStream<u8>>,
    #[rustradio(default)]
    busy_state: Option<u8>,
}

impl KissDecode {
    /// Attach channel state: zero means clear, any nonzero value means busy.
    pub fn busy(&mut self) -> Result<ReadStream<u8>> {
        if self.busy.is_some() {
            return Err(Error::msg("KissDecode busy output already created"));
        }
        let (tx, rx) = crate::stream::new_stream()?;
        self.busy = Some(tx);
        Ok(rx)
    }
}

impl Block for KissDecode {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            if let Some(ref busy) = self.busy
                && self.busy_state.is_some()
            {
                let mut o = busy.write_buf()?;
                if o.is_empty() {
                    return Ok(BlockRet::WaitForStream(busy, 1));
                }
                let b = self.busy_state.take().expect("impossible: we checked");
                o.slice()[0] = b;
                o.produce(1, vec![]);
            }
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
            let cmd = out.remove(0);
            let port = (cmd >> 4) & 0xf;
            match cmd & 0xf {
                // Data.
                KISS_DATA => {}
                // Set hardware, channel busy.
                KISS_SET_HARDWARE
                    if out.len() == 2 && out[0] == MESHCORE_CHANNEL_BUSY && out[1] < 2 =>
                {
                    // We don't get here if there's a `busy` output and it's
                    // already set, so no data is lost by blindly overwriting.
                    self.busy_state = Some(out[1]);
                    continue;
                }
                _ => {
                    debug!("KissDecode: non-data/busy packet: {cmd:02x} {out:02x?}");
                    continue;
                }
            }

            // Count encoded payload bytes without the possibly escaped command.
            let command_bytes = if x[0] == KISS_FESC { 2 } else { 1 };
            let zeros = tags.partition_point(|tag| tag.pos() == 0);
            tags.splice(
                zeros..zeros,
                [
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
                ],
            );
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
            let (i, _tags) = self.src.read_buf()?;
            if i.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            let old_state = std::mem::replace(&mut self.state, FrameState::Unsynced);
            self.state = match old_state {
                FrameState::Unsynced => {
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

/// Encode packet PDUs as KISS data frames.
///
/// `KissEncode:port` tags select the data port (default zero). Other packet
/// tags retain their ordering. An optional busy stream reports channel state
/// independently of packets using MeshCore SetHardware ChannelBusy frames on
/// port zero: `C0 06 8E state C0`. Busy-input tags are discarded.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, noeof)]
pub struct KissEncode {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(in, default)]
    busy: Option<ReadStream<u8>>,
    #[rustradio(out, sample_rate = "self.src.sample_rate()")]
    dst: NCWriteStream<Vec<u8>>,
    #[rustradio(default)]
    last_busy: Option<bool>,
    // After a control frame fills the output, give a waiting packet the next
    // slot rather than repeatedly filling it with more control transitions.
    #[rustradio(default)]
    packet_turn: bool,
}

impl KissEncode {
    /// Attach channel state: zero means clear, any nonzero value means busy.
    ///
    /// The first sample and subsequent state changes produce status frames.
    /// Both inputs advance independently; completion waits for both to drain.
    /// With two live, empty inputs the scheduler polls for either input.
    #[must_use]
    pub fn with_busy(mut self, busy: ReadStream<u8>) -> Self {
        self.busy = Some(busy);
        self.last_busy = None;
        self.packet_turn = false;
        self
    }

    fn encode_packet(&self, x: Vec<u8>, mut tags: Vec<Tag>) {
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
        let out = escape(&x, (port as u8) << 4);
        tags.retain(|tag| tag.key() != ENCODE_PORT_TAG);
        let zeros = tags.partition_point(|tag| tag.pos() == 0);
        tags.splice(
            zeros..zeros,
            [
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
            ],
        );
        self.dst.push(out, tags);
    }
}

impl BlockEOF for KissEncode {
    fn eof(&mut self) -> bool {
        self.src.eof() && self.busy.as_ref().is_none_or(|busy| busy.eof())
    }
}

impl Block for KissEncode {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let Some(busy) = &self.busy else {
            // The packet-only path can wait directly on its single input.
            loop {
                if self.dst.remaining() == 0 {
                    return Ok(BlockRet::WaitForStream(&self.dst, 1));
                }
                let Some((packet, tags)) = self.src.pop() else {
                    return Ok(BlockRet::WaitForStream(&self.src, 1));
                };
                self.encode_packet(packet, tags);
            }
        };
        if self.dst.remaining() == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let mut progressed = false;
        let mut packet_sent = false;
        if self.packet_turn
            && let Some((packet, tags)) = self.src.pop()
        {
            self.encode_packet(packet, tags);
            self.packet_turn = false;
            progressed = true;
            packet_sent = true;
            if self.dst.remaining() == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
        }
        let (input, _tags) = busy.read_buf()?;
        let mut consumed = 0;
        // Bound control work so a continuous busy stream cannot starve packets.
        for &value in input.iter().take(1024) {
            let state = value != 0;
            if self.last_busy != Some(state) {
                if self.dst.remaining() == 0 {
                    break;
                }
                self.dst.push(
                    escape(&[MESHCORE_CHANNEL_BUSY, u8::from(state)], KISS_SET_HARDWARE),
                    &[],
                );
                self.last_busy = Some(state);
                self.packet_turn = true;
            }
            consumed += 1;
        }
        input.consume(consumed);
        if self.dst.remaining() == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        progressed |= consumed != 0;
        if !packet_sent && let Some((packet, tags)) = self.src.pop() {
            self.encode_packet(packet, tags);
            self.packet_turn = false;
            progressed = true;
        }
        if self.src.eof() && busy.eof() {
            Ok(BlockRet::EOF)
        } else if progressed {
            Ok(BlockRet::Again)
        } else if busy.eof() {
            Ok(BlockRet::WaitForStream(&self.src, 1))
        } else if self.src.eof() {
            Ok(BlockRet::WaitForStream(busy, 1))
        } else {
            // Waiting on either stream alone would delay the other stream's
            // independent events. Pending also keeps an empty pair alive.
            Ok(BlockRet::Pending)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::stream::new_nocopy_stream;

    #[test]
    fn busy_changes_are_normalized_and_independent_of_packets() -> Result<()> {
        let (packets, input) = new_nocopy_stream();
        let (states, busy) = crate::stream::new_stream()?;
        let (encoder, output) = KissEncode::new(input);
        let mut encoder = encoder.with_busy(busy);
        assert!(matches!(encoder.work()?, BlockRet::Pending));
        packets.push(vec![0xc0], &[]);
        assert!(matches!(encoder.work()?, BlockRet::Again));
        assert_eq!(output.pop().unwrap().0, [0xc0, 0, 0xdb, 0xdc, 0xc0]);
        drop(packets);
        assert!(!encoder.eof());
        let mut window = states.write_buf()?;
        window.slice()[..6].copy_from_slice(&[0, 0, 1, 2, 255, 0]);
        window.produce(6, &[]);
        assert!(matches!(encoder.work()?, BlockRet::Again));
        for state in [0, 1, 0] {
            let (frame, tags) = output.pop().unwrap();
            assert_eq!(frame, [0xc0, 6, 0x8e, state, 0xc0]);
            assert!(tags.is_empty());
        }
        assert!(output.pop().is_none());
        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        drop(states);
        assert!(encoder.eof());
        assert!(matches!(encoder.work()?, BlockRet::EOF));
        Ok(())
    }

    #[test]
    fn busy_backpressure_preserves_transitions_and_packets() -> Result<()> {
        let (packets, input) = new_nocopy_stream();
        let (states, busy) = crate::stream::new_stream()?;
        let (encoder, output) = KissEncode::new(input);
        let mut encoder = encoder.with_busy(busy);
        let capacity = encoder.dst.remaining();
        for _ in 0..capacity {
            encoder.dst.push(vec![], &[]);
        }
        packets.push(vec![42], &[]);
        let mut window = states.write_buf()?;
        window.slice()[..4].copy_from_slice(&[0, 0, 1, 0]);
        window.produce(4, &[]);
        drop(states);
        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        assert_eq!(encoder.busy.as_ref().unwrap().read_buf()?.0.len(), 4);
        output.pop().unwrap();
        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(!encoder.src.is_empty());
        assert_eq!(
            encoder.busy.as_ref().unwrap().read_buf()?.0.slice(),
            &[1, 0]
        );
        // Even with only one free slot, a packet gets a turn before more
        // queued control transitions. Neither input can monopolize output.
        assert!(output.pop().unwrap().0.is_empty());
        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(encoder.src.is_empty());
        assert_eq!(
            encoder.busy.as_ref().unwrap().read_buf()?.0.slice(),
            &[1, 0]
        );
        for _ in 2..capacity {
            assert!(output.pop().unwrap().0.is_empty());
        }
        assert_eq!(output.pop().unwrap().0, [0xc0, 6, 0x8e, 0, 0xc0]);
        assert_eq!(output.pop().unwrap().0, [0xc0, 0, 42, 0xc0]);
        assert!(matches!(encoder.work()?, BlockRet::Again));
        for state in [1, 0] {
            assert_eq!(output.pop().unwrap().0, [0xc0, 6, 0x8e, state, 0xc0]);
        }
        // An ended control stream does not close a live packet input.
        assert!(!encoder.eof());
        assert!(matches!(encoder.work()?, BlockRet::WaitForStream(_, 1)));
        drop(packets);
        assert!(encoder.eof());
        Ok(())
    }

    #[test]
    fn continuous_busy_samples_do_not_starve_packets() -> Result<()> {
        let (packets, input) = new_nocopy_stream();
        let (states, busy) = crate::stream::new_stream()?;
        let (encoder, output) = KissEncode::new(input);
        let mut encoder = encoder.with_busy(busy);
        let mut window = states.write_buf()?;
        window.slice()[..2048].fill(1);
        window.produce(2048, &[]);
        packets.push(vec![42], &[]);
        assert!(matches!(encoder.work()?, BlockRet::Again));
        assert_eq!(output.pop().unwrap().0, [0xc0, 6, 0x8e, 1, 0xc0]);
        assert_eq!(output.pop().unwrap().0, [0xc0, 0, 42, 0xc0]);
        assert_eq!(encoder.busy.as_ref().unwrap().read_buf()?.0.len(), 1024);
        Ok(())
    }

    #[cfg(not(feature = "wasm"))]
    #[test]
    fn busy_encoder_finishes_in_single_and_multi_thread_graphs() -> Result<()> {
        use crate::blocks::{PduToStream, VectorSink};
        use crate::graph::{Graph, GraphRunner};
        fn check(mut graph: impl GraphRunner) -> Result<()> {
            let (packets, input) = new_nocopy_stream();
            packets.push(vec![42], &[]);
            drop(packets);
            let (encoder, frames) = KissEncode::new(input);
            graph.add(Box::new(
                encoder.with_busy(ReadStream::from_slice(&[0, 1, 0])),
            ));
            let (flatten, wire) = PduToStream::new(frames)?;
            graph.add(Box::new(flatten));
            let sink = VectorSink::new(wire, 100);
            let hook = sink.hook();
            graph.add(Box::new(sink));
            graph.run()?;
            assert_eq!(
                hook.data().samples(),
                &[
                    0xc0, 6, 0x8e, 0, 0xc0, 0xc0, 6, 0x8e, 1, 0xc0, 0xc0, 6, 0x8e, 0, 0xc0, 0xc0,
                    0, 42, 0xc0
                ]
            );
            Ok(())
        }
        check(Graph::new())?;
        #[cfg(not(feature = "wasm"))]
        check(crate::mtgraph::MTGraph::new())?;
        Ok(())
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_busy_encoder_wakes_for_both_inputs_and_drains() -> Result<()> {
        use crate::agraph::AsyncGraph;
        use crate::blocks::{PduToStream, VectorSink};
        use crate::graph::GraphRunner;
        let (packets, input) = new_nocopy_stream();
        let (states, busy) = crate::stream::new_stream()?;
        let (encoder, frames) = KissEncode::new(input);
        let mut graph = AsyncGraph::new();
        graph.add(Box::new(encoder.with_busy(busy)));
        let (flatten, wire) = PduToStream::new(frames)?;
        graph.add(Box::new(flatten));
        let sink = VectorSink::new(wire, 100);
        let hook = sink.hook();
        graph.add(Box::new(sink));
        let producer = async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let mut window = states.write_buf()?;
            window.slice()[..2].copy_from_slice(&[0, 1]);
            window.produce(2, &[]);
            drop(states);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            packets.push(vec![42], &[]);
            drop(packets);
            Ok::<(), crate::Error>(())
        };
        let (result, produced) = tokio::join!(graph.run_async(), producer);
        result?;
        produced?;
        assert_eq!(
            hook.data().samples(),
            &[
                0xc0, 6, 0x8e, 0, 0xc0, 0xc0, 6, 0x8e, 1, 0xc0, 0xc0, 0, 42, 0xc0
            ]
        );
        Ok(())
    }

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
            &[
                Tag::new(0, "foobar", TagValue::String("baz".to_string())),
                Tag::new(1, "first", TagValue::Bool(true)),
                Tag::new(1, "second", TagValue::Bool(false)),
                Tag::new(4, "later", TagValue::U64(42)),
            ],
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
                Tag::new(1, "first", TagValue::Bool(true)),
                Tag::new(1, "second", TagValue::Bool(false)),
                Tag::new(4, "later", TagValue::U64(42)),
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
                Tag::new(1, "first", TagValue::Bool(true)),
                Tag::new(1, "second", TagValue::Bool(false)),
                Tag::new(4, "later", TagValue::U64(42)),
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
                Tag::new(1, "first", TagValue::Bool(true)),
                Tag::new(1, "second", TagValue::Bool(false)),
                Tag::new(4, "later", TagValue::U64(42)),
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
    fn frame_survives_every_input_split() -> Result<()> {
        let wire = [KISS_FEND, 0, 1, KISS_FESC, KISS_TFEND, 3, KISS_FEND];
        for split in 0..=wire.len() {
            let (tx, rx) = crate::stream::new_stream()?;
            let (mut frame, output) = KissFrame::new(rx);
            for chunk in [&wire[..split], &wire[split..]] {
                let mut window = tx.write_buf()?;
                window.fill_from_slice(chunk);
                window.produce(chunk.len(), &[]);
                assert!(matches!(frame.work()?, BlockRet::WaitForStream(_, 1)));
                assert!(matches!(frame.work()?, BlockRet::WaitForStream(_, 1)));
            }
            assert_eq!(
                output.pop().expect("complete frame").0,
                wire[1..wire.len() - 1],
                "split at {split}",
            );
            assert!(output.pop().is_none());
        }
        Ok(())
    }

    #[test]
    fn frames_share_a_delimiter_across_single_byte_batches() -> Result<()> {
        let (tx, rx) = crate::stream::new_stream()?;
        let (mut frame, output) = KissFrame::new(rx);
        for byte in [KISS_FEND, 0, 1, KISS_FEND, 0, 2, KISS_FEND] {
            let mut window = tx.write_buf()?;
            window.fill_from_slice(&[byte]);
            window.produce(1, &[]);
            assert!(matches!(frame.work()?, BlockRet::WaitForStream(_, 1)));
        }
        assert_eq!(output.pop().expect("first frame").0, [0, 1]);
        assert_eq!(output.pop().expect("second frame").0, [0, 2]);
        assert!(output.pop().is_none());
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
