use prost::Message;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

use super::{IqSample, IqServer, TAG_KINDS, advance, err, limits_valid, proto, tag_to_wire};
use crate::Result;
use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::ReadStream;

/// Sample allocations return here after the transport encodes their frame.
#[derive(Clone)]
pub(super) struct SamplePool {
    buffers: Arc<Mutex<Vec<Vec<u8>>>>,
    capacity: usize,
}
impl SamplePool {
    fn new(capacity: usize) -> Self {
        Self {
            buffers: Default::default(),
            capacity,
        }
    }
    fn take(&self, bytes: usize) -> Vec<u8> {
        let mut buffer = self
            .buffers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop()
            .unwrap_or_default();
        buffer.clear();
        buffer.reserve(bytes);
        buffer
    }
    pub fn put(&self, mut buffer: Vec<u8>) {
        buffer.clear();
        let mut buffers = self.buffers.lock().unwrap_or_else(|p| p.into_inner());
        if buffers.len() < self.capacity {
            buffers.push(buffer);
        }
    }
}
// One named stream shared by the graph sink and its current network session.
// The graph takes a short lock and never awaits socket or credit readiness.
pub(super) struct Resource {
    pub state: Mutex<ResourceState>,
    pub notify: Notify,
    pub pool: SamplePool,
    pub blocking: bool,
    pub limits: proto::Limits,
}
pub(super) struct ResourceState {
    pub description: proto::StreamDescription,
    // Graph samples consumed across all connections, including discarded data.
    // A reconnect uses this as its description's source_sample_offset.
    pub position: u64,
    pub active: Option<Session>,
    pub eof: bool,
    pub stopped: bool,
    // Distinguishes reconnects so an old connection cannot detach a new one.
    pub generation: u64,
}
pub(super) struct Session {
    pub generation: u64,
    pub limits: proto::Limits,
    // Receiver's exclusive frame sequence limit. Zero means no initial credit.
    pub send_limit: u64,
    // Cursors for frames submitted by the transport, starting at zero per session.
    pub next_sequence: u64,
    pub next_sample: u64,
    // Samples consumed by the sink in this session, including queued and lost
    // samples. This can lead next_sample while the transport is backpressured.
    pub cursor: u64,
    // FIFO bounded by max_in_flight_frames; frame sequences are assigned on send.
    pub queue: VecDeque<proto::frame::Body>,
    // A separate, constant-size accumulator for dropped samples. Queue this gap
    // before retaining new samples so the receiver's sample cursor stays exact.
    pub lost: Option<proto::Gap>,
    // Set per connection, including reconnects; cleared only after retaining
    // the first sample chunk so gaps cannot discard the sink identification.
    pub identity_pending: bool,
    pub end_sent: bool,
}
impl Resource {
    pub fn lock(&self) -> MutexGuard<'_, ResourceState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}
/// Configure an experimental native graph sink.
pub struct IqStreamSinkBuilder<T: IqSample> {
    src: ReadStream<T>,
    server: IqServer,
    name: String,
    rate: f64,
    blocking: bool,
    limits: proto::Limits,
}
impl<T: IqSample> IqStreamSinkBuilder<T> {
    /// Blocking preserves input when queues fill and waits while disconnected.
    /// Nonblocking drops new samples and reports exact gaps to connected clients.
    #[must_use]
    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }
    /// Upper bounds selected during negotiation.
    #[must_use]
    pub fn limits(mut self, limits: proto::Limits) -> Self {
        self.limits = limits;
        self
    }
    /// Validate configuration and register this named stream exactly once.
    pub fn build(self) -> Result<IqStreamSink<T>> {
        limits_valid(&self.limits)?;
        if self.name.is_empty() || !self.rate.is_finite() || self.rate <= 0.0 {
            return Err(err("invalid IQ stream name or sample rate"));
        }
        let description = proto::StreamDescription {
            encoding: Some(T::encoding()),
            sample_rate_hz: self.rate,
            tag_kinds: TAG_KINDS.to_vec(),
            source_id: self.name.clone(),
            ..Default::default()
        };
        if description.encoded_len() + 64 > super::MAX_CONTROL {
            return Err(err("IQ description byte limit"));
        }
        let resource = Arc::new(Resource {
            state: Mutex::new(ResourceState {
                description,
                position: 0,
                active: None,
                eof: false,
                stopped: false,
                generation: 0,
            }),
            pool: SamplePool::new(self.limits.max_in_flight_frames as usize),
            notify: Notify::new(),
            blocking: self.blocking,
            limits: self.limits,
        });
        self.server.register(self.name, resource.clone())?;
        Ok(IqStreamSink {
            src: self.src,
            resource,
            done: false,
        })
    }
}
/// Native server sink shared by gRPC and WebSocket download clients.
///
/// Supports one active client per named stream, without replay. All retained
/// samples and ordinary scalar tags are preserved in blocking mode. Nonblocking
/// overflow drops newly arriving samples and their tags, leaving queued samples
/// intact. After a gap, receivers must regard persistent tag state as unknown.
/// The two reserved local gap keys cannot be used as ordinary input tags.
///
/// Each connection's first retained sample carries string tags identifying the
/// sender: `rustradio.software` ("rustradio"), `rustradio.version` (crate version),
/// and `rustradio.git_version` (build-time Git version, omitted if empty). These
/// precede input tags at the same position and count toward the frame byte limit.
/// `rustradio.iq.absolute_sample_index` carries that sample's absolute sink
/// position as a uint64, including samples discarded while disconnected or
/// overflowing. Empty streams carry no identification tags.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct IqStreamSink<T: IqSample> {
    #[rustradio(in)]
    src: ReadStream<T>,
    resource: Arc<Resource>,
    done: bool,
}
impl<T: IqSample> IqStreamSink<T> {
    /// Create a builder. A server can register many independently named streams.
    pub fn builder(
        src: ReadStream<T>,
        server: &IqServer,
        name: impl Into<String>,
        sample_rate_hz: f64,
    ) -> IqStreamSinkBuilder<T> {
        IqStreamSinkBuilder {
            src,
            server: server.clone(),
            name: name.into(),
            rate: sample_rate_hz,
            blocking: true,
            limits: super::StreamOptions::default().limits,
        }
    }
}
impl<T: IqSample> BlockEOF for IqStreamSink<T> {
    fn eof(&mut self) -> bool {
        self.done
    }
}
impl<T: IqSample> IqStreamSink<T> {
    fn work_inner(&mut self) -> Result<BlockRet<'_>> {
        if self.done {
            return Ok(BlockRet::EOF);
        }
        let mut state = self.resource.lock();
        if state.stopped {
            return Err(err("IQ server stopped"));
        }
        if let Some(session) = state.active.as_mut() {
            // Older retained frames precede the loss; any new chunk must follow
            // it. The gap itself uses one queue slot and one frame of credit.
            if session.queue.len() < session.limits.max_in_flight_frames as usize
                && let Some(gap) = session.lost.take()
            {
                session.queue.push_back(proto::frame::Body::Gap(gap));
                self.resource.notify.notify_one();
            }
        }
        let (input, tags) = self.src.read_buf()?;
        if input.is_empty() {
            drop(input);
            if self.src.eof() {
                state.eof = true;
                self.resource.notify.notify_one();
                if state.active.is_none() {
                    self.done = true;
                    return Ok(BlockRet::EOF);
                }
                return Ok(BlockRet::Pending);
            }
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        // Capture before borrowing the session: this is the absolute position
        // of the current input, including samples lost before this connection.
        let absolute_position = state.position;
        let n = if let Some(session) = state.active.as_mut() {
            if session.end_sent {
                return Err(err("IQ input after End"));
            }
            if session.queue.len() >= session.limits.max_in_flight_frames as usize {
                if self.resource.blocking {
                    return Ok(BlockRet::Pending);
                }
                // Drop new input rather than evicting queued frames. Adjacent
                // overflow spans merge without allocating an unbounded queue.
                let count = input.len() as u64;
                let end = advance(session.cursor, count)?;
                session.lost = Some(match session.lost.take() {
                    Some(gap) => proto::Gap {
                        sample_count: advance(gap.sample_count, count)?,
                        ..gap
                    },
                    None => proto::Gap {
                        first_sample: session.cursor,
                        sample_count: count,
                        reason: "sink overflow".into(),
                    },
                });
                session.cursor = end;
                input.len()
            } else {
                let first = session.cursor;
                // Include queued frames when predicting this frame's sequence,
                // because protobuf varint overhead grows with the sequence value.
                let sequence = advance(session.next_sequence, session.queue.len() as u64)?;
                if sequence == u64::MAX {
                    return Err(err("IQ frame sequence exhausted"));
                }
                let mut n = input
                    .len()
                    .min(session.limits.max_frame_bytes as usize / T::size());
                let mut samples = self.resource.pool.take(n * T::size());
                for sample in &input.slice()[..n] {
                    sample.serialize_into(&mut samples);
                }
                advance(first, n as u64)?;
                let mut wire_tags = if session.identity_pending {
                    identity_tags(first, absolute_position, env!("GIT_VERSION"))
                } else {
                    Vec::new()
                };
                for tag in tags.iter().take_while(|t| t.pos() < n) {
                    wire_tags.push(tag_to_wire(tag, first)?);
                }
                // Account for all protobuf overhead. A sample and its tags are
                // indivisible, so find the largest fitting prefix by byte size.
                let fits = |count: usize| {
                    let count_tags =
                        wire_tags.partition_point(|t| t.sample_index < first + count as u64);
                    let mut chunk_size = scalar_size(first) + scalar_size(count as u64);
                    let bytes = count * T::size();
                    if bytes > 0 {
                        chunk_size += 1 + varint_size(bytes as u64) + bytes;
                    }
                    for tag in &wire_tags[..count_tags] {
                        let size = tag.encoded_len();
                        chunk_size += 1 + varint_size(size as u64) + size;
                    }
                    let frame_size =
                        scalar_size(sequence) + 1 + varint_size(chunk_size as u64) + chunk_size;
                    count_tags <= 4096 && frame_size <= session.limits.max_frame_bytes as usize
                };
                if !fits(n) {
                    let (mut low, mut high) = (0, n);
                    while low < high {
                        let middle = (low + high).div_ceil(2);
                        if fits(middle) {
                            low = middle;
                        } else {
                            high = middle - 1;
                        }
                    }
                    n = low;
                }
                if n == 0 {
                    return Err(err("IQ frame limit cannot fit one sample and its tags"));
                }
                let end = advance(first, n as u64)?;
                samples.truncate(n * T::size());
                let tags = wire_tags
                    .into_iter()
                    .take_while(|t| t.sample_index < end)
                    .collect();
                session
                    .queue
                    .push_back(proto::frame::Body::Chunk(proto::SampleChunk {
                        first_sample: first,
                        sample_count: n as u32,
                        samples,
                        tags,
                    }));
                session.cursor = end;
                session.identity_pending = false;
                n
            }
        } else {
            if self.resource.blocking {
                return Ok(BlockRet::Pending);
            }
            input.len()
        };
        state.position = advance(state.position, n as u64)?;
        input.consume(n);
        self.resource.notify.notify_one();
        Ok(BlockRet::Again)
    }
}
impl<T: IqSample> Block for IqStreamSink<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let resource = self.resource.clone();
        let result = self.work_inner();
        if result.is_err() {
            let mut state = resource.lock();
            state.eof = true;
            state.active = None;
            drop(state);
            resource.notify.notify_one();
        }
        result
    }
}
impl<T: IqSample> Drop for IqStreamSink<T> {
    fn drop(&mut self) {
        let mut state = self.resource.lock();
        state.eof = true;
        // A graph dropped before completing the sink must abort its session.
        if !self.done {
            state.active = None;
        }
        drop(state);
        self.resource.notify.notify_one();
    }
}

fn varint_size(n: u64) -> usize {
    ((64 - n.leading_zeros()).max(1) as usize).div_ceil(7)
}
fn scalar_size(n: u64) -> usize {
    if n == 0 { 0 } else { 1 + varint_size(n) }
}

// Ordinary sample tags keep identification visible to graph and file adapters.
// Take the Git value explicitly so the empty build-version case is testable.
pub(super) fn identity_tags(
    sample_index: u64,
    absolute_position: u64,
    git_version: &str,
) -> Vec<proto::Tag> {
    let mut tags: Vec<_> = [
        ("rustradio.software", "rustradio"),
        ("rustradio.version", env!("CARGO_PKG_VERSION")),
        ("rustradio.git_version", git_version),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| proto::Tag {
        sample_index,
        key: key.into(),
        value: Some(proto::TagValue {
            kind: Some(proto::tag_value::Kind::StringValue(value.into())),
        }),
        source_id: None,
    })
    .collect();
    tags.push(proto::Tag {
        sample_index,
        key: super::ABSOLUTE_SAMPLE_INDEX.into(),
        value: Some(proto::TagValue {
            kind: Some(proto::tag_value::Kind::Uint64Value(absolute_position)),
        }),
        source_id: None,
    });
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identification_omits_empty_git_version() {
        let tags = identity_tags(42, u64::MAX, "");
        assert_eq!(tags.len(), 3);
        for (tag, (key, value)) in tags.iter().zip([
            ("rustradio.software", "rustradio"),
            ("rustradio.version", env!("CARGO_PKG_VERSION")),
        ]) {
            assert_eq!(tag.sample_index, 42);
            assert_eq!(tag.key, key);
            assert_eq!(
                tag.value.as_ref().unwrap().kind,
                Some(proto::tag_value::Kind::StringValue(value.into()))
            );
        }
        assert_eq!(tags[2].sample_index, 42);
        assert_eq!(tags[2].key, super::super::ABSOLUTE_SAMPLE_INDEX);
        assert_eq!(
            tags[2].value.as_ref().unwrap().kind,
            Some(proto::tag_value::Kind::Uint64Value(u64::MAX))
        );
        let tags = identity_tags(42, u64::MAX, "revision");
        assert_eq!(tags.len(), 4);
        assert_eq!(tags[2].key, "rustradio.git_version");
        assert_eq!(
            tags[2].value.as_ref().unwrap().kind,
            Some(proto::tag_value::Kind::StringValue("revision".into()))
        );
    }
}
