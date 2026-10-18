use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    Decoded, GAP_SAMPLES, IqSample, SAMPLE_INDEX, TAG_KINDS, advance, err, limits_valid, proto,
    reserved, value_from_wire, value_kind,
};
use crate::Result;
use crate::block::{Block, BlockEOF, BlockRet};
use crate::stream::{
    NCReadStream, NCWriteStream, ReadStream, StreamWait, Tag, TagValue, WriteStream,
    new_nocopy_stream, new_stream,
};
use async_channel::{Receiver, Sender};
use prost::Message;

/// Download policy and bounds. ALLOW_GAPS requires a gap-aware graph.
#[derive(Clone, Debug)]
pub struct StreamOptions {
    /// Receiver's requested frame and credit bounds.
    pub limits: proto::Limits,
    /// Must match the sink: LOSSLESS for blocking, ALLOW_GAPS otherwise.
    pub loss_policy: proto::LossPolicy,
}
impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            limits: proto::Limits {
                max_frame_bytes: 256 * 1024,
                max_in_flight_frames: 8,
            },
            loss_policy: proto::LossPolicy::Lossless,
        }
    }
}
impl StreamOptions {
    /// Construct an Open envelope for the exact graph sample type.
    pub fn open<T: IqSample>(&self, source: impl Into<String>) -> Result<proto::ClientMessage> {
        self.open_encoding(source, T::encoding())
    }
    /// Construct an Open envelope preserving individual UTF-8 string messages.
    pub fn open_strings(&self, source: impl Into<String>) -> Result<proto::ClientMessage> {
        self.open_encoding(source, super::string_encoding())
    }
    fn open_encoding(
        &self,
        source: impl Into<String>,
        encoding: proto::Encoding,
    ) -> Result<proto::ClientMessage> {
        limits_valid(&self.limits)?;
        if self.loss_policy == proto::LossPolicy::Unspecified {
            return Err(err("unspecified IQ loss policy"));
        }
        let source = source.into();
        if source.is_empty() {
            return Err(err("empty IQ source identifier"));
        }
        let message = proto::ClientMessage {
            body: Some(proto::client_message::Body::Open(proto::Open {
                protocol_version: 1,
                limits: Some(self.limits.clone()),
                loss_policy: self.loss_policy as i32,
                completion_mode: proto::CompletionMode::Accepted as i32,
                operation: Some(proto::open::Operation::Download(proto::Download {
                    source,
                    accepted_encodings: vec![encoding],
                    accepted_tag_kinds: TAG_KINDS.to_vec(),
                    ..Default::default()
                })),
            })),
        };
        if message.encoded_len() > super::MAX_CONTROL {
            return Err(err("IQ Open byte limit"));
        }
        Ok(message)
    }
}
/// Session outcome; End alone does not imply success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    /// Samples are arriving.
    Receiving,
    /// End is accepted; waiting for final acknowledgement and transport close.
    Completing,
    /// Both peers and the transport completed successfully.
    Complete,
    /// Connection/protocol failure.
    Failed(String),
    /// The source was dropped or its graph consumer stopped before completion.
    Cancelled,
}
struct State {
    started: proto::Started,
    status: SourceStatus,
    // Expected arrival cursors. Gaps advance samples as well as frame sequence.
    next_sequence: u64,
    next_sample: u64,
    // Frames fully accepted by the graph, and the exclusive sequence limit
    // already granted to the sender. Receiving a frame alone returns no credit.
    retired: u64,
    grant: u64,
    end: Option<proto::End>,
    // Completion is requested after draining graph output, sent by the control
    // task, acknowledged by the server, then confirmed by clean transport close.
    complete_requested: bool,
    complete_sent: bool,
    cancel_sent: bool,
    server_complete: bool,
    lost_samples: u64,
    // Merge adjacent gaps until a retained sample can carry their marker tags.
    // If End arrives first, the application can inspect this through the handle.
    trailing_gap: Option<proto::Gap>,
}
/// Bounded session status shared with the application.
#[derive(Clone)]
pub struct SourceHandle {
    state: Arc<Mutex<State>>,
    notify: Sender<()>,
}
impl SourceHandle {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn wake(&self) {
        let _ = self.notify.try_send(());
    }
    /// Immutable negotiated metadata, including sample rate and source offset.
    pub fn description(&self) -> proto::StreamDescription {
        self.lock()
            .started
            .description
            .clone()
            .expect("validated description")
    }
    /// Current outcome.
    pub fn status(&self) -> SourceStatus {
        self.lock().status.clone()
    }
    /// Exact missing samples accepted by the graph so far.
    pub fn lost_samples(&self) -> u64 {
        self.lock().lost_samples
    }
    /// Loss after the last retained sample, including a gap immediately before EOF.
    pub fn trailing_gap(&self) -> Option<proto::Gap> {
        self.lock().trailing_gap.clone()
    }
    /// End cursors, available once End has arrived; consult status for success.
    pub fn end(&self) -> Option<proto::End> {
        self.lock().end.clone()
    }
    fn cancel(&self) {
        let mut state = self.lock();
        if matches!(
            state.status,
            SourceStatus::Receiving | SourceStatus::Completing
        ) {
            state.status = SourceStatus::Cancelled;
        }
        drop(state);
        self.wake();
    }
    fn fail(&self, message: impl Into<String>) {
        let mut state = self.lock();
        if matches!(
            state.status,
            SourceStatus::Receiving | SourceStatus::Completing
        ) {
            state.status = SourceStatus::Failed(message.into());
        }
        drop(state);
        self.wake();
    }
    fn retire(&self) -> Result<()> {
        let mut s = self.lock();
        s.retired = advance(s.retired, 1)?;
        drop(s);
        self.wake();
        Ok(())
    }
}

/// Transport half of a source, shared by native and browser adapters.
///
/// Feed budget-checked server envelopes into `accept`, write `next_control`
/// envelopes, and call `finish` only after successful transport completion.
/// Socket callbacks must also wake the browser graph after arrivals or failure.
pub struct SourceTransport {
    handle: SourceHandle,
    frames: Sender<proto::Frame>,
    notify: Receiver<()>,
    // Fixed numeric sample size, or None for one variable-length UTF-8 string.
    sample_size: Option<usize>,
}
impl SourceTransport {
    /// Clone the application status handle.
    pub fn handle(&self) -> SourceHandle {
        self.handle.clone()
    }
    /// Validate a received message before queueing it into bounded graph storage.
    pub fn accept(&self, received: Decoded<proto::ServerMessage>) -> Result<()> {
        use proto::server_message::Body;
        let mut s = self.handle.lock();
        if !matches!(s.status, SourceStatus::Receiving | SourceStatus::Completing) {
            return Err(err("message after terminal IQ status"));
        }
        match received.message.body {
            Some(Body::Frame(frame)) => {
                let limits = s.started.limits.as_ref().expect("validated limits");
                if s.end.is_some()
                    || frame.sequence != s.next_sequence
                    || frame.sequence >= s.grant
                    || received
                        .frame_bytes
                        .is_none_or(|n| n > limits.max_frame_bytes as usize)
                {
                    return Err(err("invalid IQ frame sequence, credit, or size"));
                }
                let count = match frame.body.as_ref() {
                    Some(proto::frame::Body::Chunk(chunk)) => {
                        let end = advance(chunk.first_sample, u64::from(chunk.sample_count))?;
                        if chunk.first_sample != s.next_sample
                            || chunk.sample_count == 0
                            || match self.sample_size {
                                Some(size) => {
                                    chunk.samples.len()
                                        != (chunk.sample_count as usize)
                                            .checked_mul(size)
                                            .ok_or_else(|| err("sample byte overflow"))?
                                }
                                None => {
                                    chunk.sample_count != 1
                                        || std::str::from_utf8(&chunk.samples).is_err()
                                }
                            }
                            || chunk.tags.len() > 4096
                        {
                            return Err(err("invalid IQ sample chunk"));
                        }
                        let mut previous = chunk.first_sample;
                        let desc = s
                            .started
                            .description
                            .as_ref()
                            .expect("validated description");
                        for tag in &chunk.tags {
                            if tag.sample_index < previous
                                || tag.sample_index >= end
                                || tag.key.is_empty()
                                || reserved(&tag.key)
                                || tag.source_id.is_some()
                            {
                                return Err(err("invalid IQ tag position, key, or provenance"));
                            }
                            let value = tag
                                .value
                                .as_ref()
                                .ok_or_else(|| err("missing IQ tag value"))?;
                            if !desc.tag_kinds.contains(&value_kind(value)?) {
                                return Err(err("undeclared IQ tag kind"));
                            }
                            previous = tag.sample_index;
                        }
                        u64::from(chunk.sample_count)
                    }
                    Some(proto::frame::Body::Gap(gap)) => {
                        if s.started.loss_policy != proto::LossPolicy::AllowGaps as i32
                            || gap.first_sample != s.next_sample
                            || gap.sample_count == 0
                        {
                            return Err(err("invalid IQ gap"));
                        }
                        gap.sample_count
                    }
                    None => return Err(err("missing IQ frame body")),
                };
                let next_sample = advance(s.next_sample, count)?;
                let next_sequence = advance(s.next_sequence, 1)?;
                self.frames
                    .try_send(frame)
                    .map_err(|_| err("IQ receive window exceeded"))?;
                s.next_sample = next_sample;
                s.next_sequence = next_sequence;
            }
            Some(Body::End(end)) => {
                if s.end.is_some()
                    || s.grant == 0
                    || end.next_sample != s.next_sample
                    || end.next_sequence != s.next_sequence
                    || !end.tags.is_empty()
                {
                    return Err(err("invalid IQ End"));
                }
                s.end = Some(end);
            }
            Some(Body::Complete(complete)) => {
                let end = s
                    .end
                    .as_ref()
                    .ok_or_else(|| err("IQ Complete before End"))?;
                if !s.complete_sent
                    || s.server_complete
                    || complete.next_sequence != end.next_sequence
                    || complete.next_sample != end.next_sample
                    || complete.completion_mode != proto::CompletionMode::Accepted as i32
                {
                    return Err(err("invalid IQ Complete"));
                }
                s.server_complete = true;
            }
            Some(Body::Failure(failure)) => {
                if !(1..=16).contains(&failure.grpc_status_code) {
                    return Err(err("invalid IQ failure code"));
                }
                return Err(err(format!(
                    "IQ peer failure {}: {}",
                    failure.grpc_status_code, failure.message
                )));
            }
            _ => return Err(err("unexpected IQ server message")),
        }
        Ok(())
    }
    /// Next coalesced client control envelope. Records grants before socket writes.
    /// Emits cancellation once, then returns None for terminal sources.
    pub async fn next_control(&self) -> Option<proto::ClientMessage> {
        loop {
            {
                let mut s = self.handle.lock();
                let body = if s.status == SourceStatus::Cancelled {
                    if s.cancel_sent {
                        return None;
                    }
                    s.cancel_sent = true;
                    Some(proto::client_message::Body::Cancel(proto::Cancel {
                        reason: "source dropped".into(),
                    }))
                } else if matches!(s.status, SourceStatus::Failed(_) | SourceStatus::Complete) {
                    return None;
                } else if s.complete_requested && !s.complete_sent {
                    s.complete_sent = true;
                    let end = s.end.as_ref().expect("completion requires End");
                    Some(proto::client_message::Body::Complete(proto::Complete {
                        next_sequence: end.next_sequence,
                        next_sample: end.next_sample,
                        completion_mode: proto::CompletionMode::Accepted as i32,
                    }))
                } else if !s.complete_sent {
                    // Cumulative credit is retired frames + the negotiated window.
                    // Saturation stops granting at u64::MAX without wrapping;
                    // sequence and sample cursors themselves use checked addition.
                    let desired = s.retired.saturating_add(u64::from(
                        s.started
                            .limits
                            .as_ref()
                            .expect("validated limits")
                            .max_in_flight_frames,
                    ));
                    if desired > s.grant {
                        // A fast peer may answer while the write future yields.
                        // Record the grant before handing it to the socket task.
                        s.grant = desired;
                        Some(proto::client_message::Body::FlowControl(
                            proto::FlowControl {
                                send_limit: desired,
                            },
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(body) = body {
                    return Some(proto::ClientMessage { body: Some(body) });
                }
            }
            if self.notify.recv().await.is_err() {
                return None;
            }
        }
    }
    /// Mark successful transport closure, requiring final server Complete.
    pub fn finish(&self) -> Result<()> {
        let mut s = self.handle.lock();
        if !s.server_complete || s.status != SourceStatus::Completing {
            return Err(err("IQ transport closed without successful final Complete"));
        }
        s.status = SourceStatus::Complete;
        drop(s);
        self.handle.wake();
        Ok(())
    }
    /// Record a terminal error and wake a native control writer.
    pub fn fail(&self, message: impl Into<String>) {
        self.handle.fail(message);
    }
}

impl Drop for SourceTransport {
    fn drop(&mut self) {
        self.handle.fail("IQ transport stopped before completion");
    }
}

/// Download source for `Float`, `Complex`, or `u8` samples.
///
/// Gaps produce [`super::GAP_SAMPLES`] and [`super::SAMPLE_INDEX`] on the next
/// retained sample. Stateful DSP must handle those discontinuities explicitly.
/// Terminal gaps are available on [`SourceHandle`]. No samples are zero-filled.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct IqStreamSource<T: IqSample> {
    #[rustradio(out)]
    // Take the writer after End drains so consumers see EOF while the transport
    // completes its acknowledgement handshake.
    dst: Option<WriteStream<T>>,
    rx: Receiver<proto::Frame>,
    // A partially copied chunk, its next sample offset, and its next tag index.
    // Both cursors move forward; output backpressure never rescans earlier tags.
    current: Option<(proto::SampleChunk, usize, usize)>,
    handle: SourceHandle,
    done: bool,
}
impl<T: IqSample> IqStreamSource<T> {
    /// Construct graph and transport halves after a successful Started handshake.
    /// Adapter authors must check the encoded envelope budgets before this call.
    pub fn from_started(
        started: proto::Started,
        options: &StreamOptions,
    ) -> Result<(Self, ReadStream<T>, SourceHandle, SourceTransport)> {
        let (rx, handle, transport) =
            start_source(started, options, T::encoding(), Some(T::ENCODED_SIZE.get()))?;
        let (dst, out) = new_stream()?;
        Ok((
            Self {
                dst: Some(dst),
                rx,
                current: None,
                handle: handle.clone(),
                done: false,
            },
            out,
            handle,
            transport,
        ))
    }
}
fn start_source(
    started: proto::Started,
    options: &StreamOptions,
    encoding: proto::Encoding,
    sample_size: Option<usize>,
) -> Result<(Receiver<proto::Frame>, SourceHandle, SourceTransport)> {
    limits_valid(&options.limits)?;
    let limits = started
        .limits
        .as_ref()
        .ok_or_else(|| err("missing IQ limits"))?;
    limits_valid(limits)?;
    let desc = started
        .description
        .as_ref()
        .ok_or_else(|| err("missing IQ description"))?;
    if limits.max_frame_bytes > options.limits.max_frame_bytes
        || limits.max_in_flight_frames > options.limits.max_in_flight_frames
        || options.loss_policy == proto::LossPolicy::Unspecified
        || started.loss_policy != options.loss_policy as i32
        || started.completion_mode != proto::CompletionMode::Accepted as i32
        || started.initial_credit.is_some()
        || desc.encoding.as_ref() != Some(&encoding)
        || !desc.sample_rate_hz.is_finite()
        || desc.sample_rate_hz <= 0.0
        || desc.uses_terminal_tags
        || desc.uses_tag_source_ids
        || !desc.opaque_codecs.is_empty()
        || desc.tag_kinds.iter().any(|kind| !TAG_KINDS.contains(kind))
        || desc
            .tag_kinds
            .iter()
            .enumerate()
            .any(|(i, k)| desc.tag_kinds[..i].contains(k))
    {
        return Err(err("unsupported IQ description or policies"));
    }
    if let Some(time) = &desc.sample_zero_time
        && (!(-62135596800..=253402300799).contains(&time.seconds)
            || !(0..1_000_000_000).contains(&time.nanos))
    {
        return Err(err("invalid IQ UTC anchor"));
    }
    let mut keys = std::collections::HashSet::new();
    for property in &desc.properties {
        if property.key.is_empty()
            || !keys.insert(&property.key)
            || !desc.tag_kinds.contains(&value_kind(
                property
                    .value
                    .as_ref()
                    .ok_or_else(|| err("missing IQ property value"))?,
            )?)
        {
            return Err(err("invalid IQ property"));
        }
    }
    let (tx, rx) = async_channel::bounded(limits.max_in_flight_frames as usize);
    let (notify_tx, notify_rx) = async_channel::bounded(1);
    let handle = SourceHandle {
        state: Arc::new(Mutex::new(State {
            started,
            status: SourceStatus::Receiving,
            next_sequence: 0,
            next_sample: 0,
            retired: 0,
            grant: 0,
            end: None,
            complete_requested: false,
            complete_sent: false,
            cancel_sent: false,
            server_complete: false,
            lost_samples: 0,
            trailing_gap: None,
        })),
        notify: notify_tx,
    };
    let transport = SourceTransport {
        handle: handle.clone(),
        frames: tx,
        notify: notify_rx,
        sample_size,
    };
    Ok((rx, handle, transport))
}
impl<T: IqSample> BlockEOF for IqStreamSource<T> {
    fn eof(&mut self) -> bool {
        self.done
    }
}
impl<T: IqSample> IqStreamSource<T> {
    fn work_inner(&mut self) -> Result<BlockRet<'_>> {
        if self.done {
            return Ok(BlockRet::EOF);
        }
        if let SourceStatus::Failed(message) = self.handle.status() {
            self.dst.take();
            return Err(err(message));
        }
        if self.dst.as_ref().is_some_and(StreamWait::closed) {
            self.dst.take();
            self.done = true;
            self.handle.cancel();
            return Ok(BlockRet::EOF);
        }
        let mut progress = false;
        loop {
            if self.current.is_none() {
                match self.rx.try_recv() {
                    Ok(frame) => match frame.body {
                        Some(proto::frame::Body::Chunk(chunk)) => {
                            self.current = Some((chunk, 0, 0))
                        }
                        Some(proto::frame::Body::Gap(gap)) => {
                            // A gap needs no sample storage. Retire it immediately
                            // so even a one-frame window can receive several gaps
                            // before the next retained sample arrives.
                            let mut s = self.handle.lock();
                            s.lost_samples = advance(s.lost_samples, gap.sample_count)?;
                            s.trailing_gap = Some(match s.trailing_gap.take() {
                                Some(previous) => proto::Gap {
                                    first_sample: previous.first_sample,
                                    sample_count: advance(previous.sample_count, gap.sample_count)?,
                                    reason: "stream discontinuity".into(),
                                },
                                None => gap,
                            });
                            drop(s);
                            self.handle.retire()?;
                            progress = true;
                            continue;
                        }
                        None => return Err(err("missing queued IQ frame body")),
                    },
                    Err(_) => {
                        let mut s = self.handle.lock();
                        if s.end.is_some() && !s.complete_requested {
                            // All queued samples are now in the graph buffer.
                            // Closing the writer lets downstream blocks drain;
                            // transport success is still required before our EOF.
                            self.dst.take();
                            s.complete_requested = true;
                            s.status = SourceStatus::Completing;
                            drop(s);
                            self.handle.wake();
                        } else if s.status == SourceStatus::Complete {
                            self.done = true;
                            return Ok(BlockRet::EOF);
                        }
                        return Ok(if progress {
                            BlockRet::Again
                        } else {
                            BlockRet::Pending
                        });
                    }
                }
            }
            let dst = self
                .dst
                .as_ref()
                .ok_or_else(|| err("IQ samples after output closure"))?;
            let (chunk, offset, tag_cursor) = self.current.as_mut().expect("current chunk");
            let mut output = dst.write_buf()?;
            if output.is_empty() {
                return Ok(BlockRet::WaitForStream(
                    self.dst.as_ref().expect("open IQ output"),
                    1,
                ));
            }
            let n = output.len().min(chunk.sample_count as usize - *offset);
            let start = advance(chunk.first_sample, *offset as u64)?;
            let end = advance(start, n as u64)?;
            let mut tags = Vec::new();
            if *offset == 0
                && let Some(gap) = self.handle.lock().trailing_gap.take()
            {
                tags.push(Tag::new(0, GAP_SAMPLES, TagValue::U64(gap.sample_count)));
                tags.push(Tag::new(0, SAMPLE_INDEX, TagValue::U64(start)));
            }
            while chunk
                .tags
                .get(*tag_cursor)
                .is_some_and(|tag| tag.sample_index < end)
            {
                let tag = &mut chunk.tags[*tag_cursor];
                tags.push(Tag::new(
                    (tag.sample_index - start) as usize,
                    std::mem::take(&mut tag.key),
                    value_from_wire(tag.value.take().expect("validated tag"))?,
                ));
                *tag_cursor += 1;
            }
            for (sample, bytes) in output.slice()[..n].iter_mut().zip(
                chunk.samples
                    [*offset * T::ENCODED_SIZE.get()..(*offset + n) * T::ENCODED_SIZE.get()]
                    .chunks_exact(T::ENCODED_SIZE.get()),
            ) {
                *sample = T::parse(bytes)?;
            }
            output.produce(n, tags);
            *offset += n;
            progress = true;
            if *offset == chunk.sample_count as usize {
                self.current = None;
                self.handle.retire()?;
            }
        }
    }
}
impl<T: IqSample> Block for IqStreamSource<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let handle = self.handle.clone();
        let result = self.work_inner();
        if let Err(error) = &result {
            handle.fail(error.to_string());
        }
        result
    }
}
impl<T: IqSample> Drop for IqStreamSource<T> {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

/// Receive individual UTF-8 strings on a bounded no-copy output.
///
/// Each item is one string, including empty strings; tags are at position zero.
/// Positions, gaps, and the negotiated rate count messages rather than bytes.
/// Gaps attach the same markers as [`IqStreamSource`]; trailing gaps remain on
/// [`SourceHandle`]. Invalid UTF-8 or chunks containing multiple strings fail
/// validation before publishing data. Each string and its tags must fit in one
/// negotiated frame. Output backpressure withholds frame credit.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, noeof)]
pub struct IqStreamStringSource {
    #[rustradio(out)]
    dst: Option<NCWriteStream<String>>,
    rx: Receiver<proto::Frame>,
    handle: SourceHandle,
    done: bool,
}
impl IqStreamStringSource {
    /// Construct graph and transport halves after a successful Started handshake.
    /// Browser adapters must check envelope budgets before calling this method,
    /// as for [`IqStreamSource::from_started`].
    pub fn from_started(
        started: proto::Started,
        options: &StreamOptions,
    ) -> Result<(Self, NCReadStream<String>, SourceHandle, SourceTransport)> {
        let (rx, handle, transport) =
            start_source(started, options, super::string_encoding(), None)?;
        let (dst, output) = new_nocopy_stream();
        Ok((
            Self {
                dst: Some(dst),
                rx,
                handle: handle.clone(),
                done: false,
            },
            output,
            handle,
            transport,
        ))
    }
    /// Connect a native gRPC string download from synchronous code.
    ///
    /// Blocks until the handshake succeeds or fails. A dedicated runtime thread
    /// drives the transport until completion, failure, or source cancellation.
    /// Returns an error if runtime startup, connection, or negotiation fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect(
        uri: impl Into<String>,
        source: impl Into<String>,
        options: StreamOptions,
    ) -> Result<(Self, NCReadStream<String>, SourceHandle)> {
        let open = options.open_strings(source)?;
        super::native::connect_source_sync(uri.into(), open, options, Self::from_started)
    }

    /// Connect a native gRPC string download. Requires a running Tokio runtime.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect_async(
        uri: impl Into<String>,
        source: impl Into<String>,
        options: StreamOptions,
    ) -> Result<(Self, NCReadStream<String>, SourceHandle)> {
        let open = options.open_strings(source)?;
        super::native::connect_source(uri, open, options, Self::from_started).await
    }
    fn work_inner(&mut self) -> Result<BlockRet<'_>> {
        if self.done {
            return Ok(BlockRet::EOF);
        }
        if let SourceStatus::Failed(message) = self.handle.status() {
            self.dst.take();
            return Err(err(message));
        }
        if self.dst.as_ref().is_some_and(StreamWait::closed) {
            self.dst.take();
            self.done = true;
            self.handle.cancel();
            return Ok(BlockRet::EOF);
        }
        let mut progress = false;
        loop {
            if self.dst.as_ref().is_some_and(|dst| dst.remaining() == 0) {
                return Ok(BlockRet::WaitForStream(self.dst.as_ref().unwrap(), 1));
            }
            let frame = match self.rx.try_recv() {
                Ok(frame) => frame,
                Err(_) => {
                    let mut state = self.handle.lock();
                    if state.end.is_some() && !state.complete_requested {
                        self.dst.take();
                        state.complete_requested = true;
                        state.status = SourceStatus::Completing;
                        drop(state);
                        self.handle.wake();
                    } else if state.status == SourceStatus::Complete {
                        self.done = true;
                        return Ok(BlockRet::EOF);
                    }
                    return Ok(if progress {
                        BlockRet::Again
                    } else {
                        BlockRet::Pending
                    });
                }
            };
            match frame.body {
                Some(proto::frame::Body::Gap(gap)) => {
                    let mut state = self.handle.lock();
                    state.lost_samples = advance(state.lost_samples, gap.sample_count)?;
                    state.trailing_gap = Some(match state.trailing_gap.take() {
                        Some(previous) => proto::Gap {
                            first_sample: previous.first_sample,
                            sample_count: advance(previous.sample_count, gap.sample_count)?,
                            reason: "stream discontinuity".into(),
                        },
                        None => gap,
                    });
                }
                Some(proto::frame::Body::Chunk(chunk)) => {
                    let dst = self
                        .dst
                        .as_ref()
                        .ok_or_else(|| err("IQ string after output closure"))?;
                    let mut tags = vec![];
                    if let Some(gap) = self.handle.lock().trailing_gap.take() {
                        tags.push(Tag::new(0, GAP_SAMPLES, TagValue::U64(gap.sample_count)));
                        tags.push(Tag::new(0, SAMPLE_INDEX, TagValue::U64(chunk.first_sample)));
                    }
                    for tag in chunk.tags {
                        tags.push(Tag::new(
                            0,
                            tag.key,
                            value_from_wire(tag.value.expect("validated tag"))?,
                        ));
                    }
                    dst.push(
                        String::from_utf8(chunk.samples).map_err(|e| err(e.to_string()))?,
                        tags,
                    );
                }
                None => return Err(err("missing queued IQ frame body")),
            }
            self.handle.retire()?;
            progress = true;
        }
    }
}
impl BlockEOF for IqStreamStringSource {
    fn eof(&mut self) -> bool {
        self.done
    }
}
impl Block for IqStreamStringSource {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let handle = self.handle.clone();
        let result = self.work_inner();
        if let Err(error) = &result {
            handle.fail(error.to_string());
        }
        result
    }
}
impl Drop for IqStreamStringSource {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn started<T: IqSample>(options: &StreamOptions) -> proto::Started {
        proto::Started {
            description: Some(proto::StreamDescription {
                encoding: Some(T::encoding()),
                sample_rate_hz: 48000.0,
                tag_kinds: TAG_KINDS.to_vec(),
                ..Default::default()
            }),
            limits: Some(options.limits.clone()),
            loss_policy: options.loss_policy as i32,
            completion_mode: proto::CompletionMode::Accepted as i32,
            initial_credit: None,
        }
    }
    fn accept(transport: &SourceTransport, body: proto::server_message::Body) -> Result<()> {
        let bytes = proto::ServerMessage { body: Some(body) }.encode_to_vec();
        transport.accept(super::super::decode_server(&bytes)?)
    }
    fn chunk(
        sequence: u64,
        first_sample: u64,
        values: &[f32],
        tags: Vec<proto::Tag>,
    ) -> proto::server_message::Body {
        let samples = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        proto::server_message::Body::Frame(proto::Frame {
            sequence,
            body: Some(proto::frame::Body::Chunk(proto::SampleChunk {
                first_sample,
                sample_count: values.len() as u32,
                samples,
                tags,
            })),
        })
    }
    fn string_started(options: &StreamOptions) -> proto::Started {
        let mut started = started::<u8>(options);
        started.description.as_mut().unwrap().encoding = Some(super::super::string_encoding());
        started
    }
    fn string_chunk(sequence: u64, bytes: Vec<u8>) -> proto::server_message::Body {
        proto::server_message::Body::Frame(proto::Frame {
            sequence,
            body: Some(proto::frame::Body::Chunk(proto::SampleChunk {
                first_sample: sequence,
                sample_count: 1,
                samples: bytes,
                tags: vec![],
            })),
        })
    }
    #[tokio::test]
    async fn string_validation_and_output_backpressure() -> Result<()> {
        let options = StreamOptions::default();
        let (mut source, output, handle, transport) =
            IqStreamStringSource::from_started(string_started(&options), &options)?;
        transport.next_control().await.unwrap();
        // Invalid frames do not advance the cursor or publish a partial message.
        assert!(accept(&transport, string_chunk(0, vec![0xff])).is_err());
        let mut bad = string_chunk(0, vec![]);
        if let proto::server_message::Body::Frame(frame) = &mut bad
            && let Some(proto::frame::Body::Chunk(chunk)) = &mut frame.body
        {
            chunk.sample_count = 2;
        }
        assert!(accept(&transport, bad).is_err());
        assert!(output.is_empty());
        let capacity = source.dst.as_ref().unwrap().remaining();
        for _ in 0..capacity {
            source.dst.as_ref().unwrap().push("queued".into(), &[]);
        }
        accept(&transport, string_chunk(0, vec![]))?;
        assert!(matches!(source.work()?, BlockRet::WaitForStream(_, 1)));
        assert_eq!(handle.lock().retired, 0);
        for _ in 0..capacity {
            output.pop().unwrap();
        }
        source.work()?;
        assert_eq!(output.pop().unwrap().0, "");
        assert_eq!(handle.lock().retired, 1);
        drop(output);
        assert!(matches!(source.work()?, BlockRet::EOF));
        assert_eq!(handle.status(), SourceStatus::Cancelled);
        Ok(())
    }
    #[tokio::test]
    async fn string_gaps_and_empty_stream_completion() -> Result<()> {
        let options = StreamOptions {
            loss_policy: proto::LossPolicy::AllowGaps,
            ..Default::default()
        };
        let (mut source, output, handle, transport) =
            IqStreamStringSource::from_started(string_started(&options), &options)?;
        transport.next_control().await.unwrap();
        accept(
            &transport,
            proto::server_message::Body::Frame(proto::Frame {
                sequence: 0,
                body: Some(proto::frame::Body::Gap(proto::Gap {
                    first_sample: 0,
                    sample_count: 3,
                    reason: "test".into(),
                })),
            }),
        )?;
        let mut frame = string_chunk(1, "雪\0🦀".as_bytes().to_vec());
        if let proto::server_message::Body::Frame(frame) = &mut frame
            && let Some(proto::frame::Body::Chunk(chunk)) = &mut frame.body
        {
            chunk.first_sample = 3;
        }
        accept(&transport, frame)?;
        source.work()?;
        let (message, tags) = output.pop().unwrap();
        assert_eq!(message, "雪\0🦀");
        assert_eq!(
            tags,
            [
                Tag::new(0, GAP_SAMPLES, TagValue::U64(3)),
                Tag::new(0, SAMPLE_INDEX, TagValue::U64(3))
            ]
        );
        assert_eq!(handle.lost_samples(), 3);

        let options = StreamOptions::default();
        let (mut source, output, handle, transport) =
            IqStreamStringSource::from_started(string_started(&options), &options)?;
        transport.next_control().await.unwrap();
        accept(
            &transport,
            proto::server_message::Body::End(proto::End::default()),
        )?;
        source.work()?;
        assert!(output.eof());
        let complete = transport.next_control().await.unwrap();
        let Some(proto::client_message::Body::Complete(complete)) = complete.body else {
            panic!("expected Complete");
        };
        accept(&transport, proto::server_message::Body::Complete(complete))?;
        transport.finish()?;
        assert!(matches!(source.work()?, BlockRet::EOF));
        assert_eq!(handle.status(), SourceStatus::Complete);
        Ok(())
    }
    #[test]
    fn string_source_rejects_numeric_encoding() {
        let options = StreamOptions::default();
        assert!(IqStreamStringSource::from_started(started::<u8>(&options), &options).is_err());
        assert!(IqStreamSource::<u8>::from_started(string_started(&options), &options).is_err());
    }
    #[tokio::test]
    async fn samples_tags_credit_and_completion() -> Result<()> {
        let options = StreamOptions::default();
        let (mut source, output, handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        let grant = transport.next_control().await.unwrap();
        assert!(matches!(
            grant.body,
            Some(proto::client_message::Body::FlowControl(
                proto::FlowControl { send_limit: 8 }
            ))
        ));
        let tags = vec![
            crate::stream::Tag::new(1, "a", TagValue::Bool(false)),
            crate::stream::Tag::new(1, "a", TagValue::U64(u64::MAX)),
        ];
        let wire_tags = tags
            .iter()
            .map(|t| super::super::tag_to_wire(t, 0))
            .collect::<Result<Vec<_>>>()?;
        accept(&transport, chunk(0, 0, &[1.0, -2.0, 3.0], wire_tags))?;
        assert!(matches!(source.work()?, BlockRet::Again));
        let (data, actual_tags) = output.read_buf()?;
        assert_eq!(data.slice(), &[1.0, -2.0, 3.0]);
        assert_eq!(actual_tags, tags);
        data.consume(3);
        assert!(matches!(
            transport.next_control().await.unwrap().body,
            Some(proto::client_message::Body::FlowControl(
                proto::FlowControl { send_limit: 9 }
            ))
        ));
        accept(
            &transport,
            proto::server_message::Body::End(proto::End {
                next_sequence: 1,
                next_sample: 3,
                tags: vec![],
            }),
        )?;
        assert!(matches!(source.work()?, BlockRet::Pending));
        assert!(output.eof());
        let complete = match transport.next_control().await.unwrap().body {
            Some(proto::client_message::Body::Complete(complete)) => complete,
            _ => panic!("expected Complete"),
        };
        assert_eq!(handle.status(), SourceStatus::Completing);
        accept(&transport, proto::server_message::Body::Complete(complete))?;
        assert!(matches!(source.work()?, BlockRet::Pending));
        transport.finish()?;
        assert!(matches!(source.work()?, BlockRet::EOF));
        assert_eq!(handle.status(), SourceStatus::Complete);
        Ok(())
    }
    #[tokio::test]
    async fn gaps_preserve_uint64_positions_and_final_loss() -> Result<()> {
        let mut options = StreamOptions {
            loss_policy: proto::LossPolicy::AllowGaps,
            ..Default::default()
        };
        options.limits.max_in_flight_frames = 1;
        let (mut source, output, handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        transport.next_control().await.unwrap();
        let missing = (1u64 << 53) + 7;
        accept(
            &transport,
            proto::server_message::Body::Frame(proto::Frame {
                sequence: 0,
                body: Some(proto::frame::Body::Gap(proto::Gap {
                    first_sample: 0,
                    sample_count: missing,
                    reason: "overflow".into(),
                })),
            }),
        )?;
        source.work()?; // A gap returns its slot without needing a following sample.
        transport.next_control().await.unwrap();
        accept(&transport, chunk(1, missing, &[0.5], vec![]))?;
        source.work()?;
        let (data, tags) = output.read_buf()?;
        assert_eq!(data.slice(), &[0.5]);
        assert_eq!(
            tags,
            vec![
                Tag::new(0, GAP_SAMPLES, TagValue::U64(missing)),
                Tag::new(0, SAMPLE_INDEX, TagValue::U64(missing))
            ]
        );
        data.consume(1);
        transport.next_control().await.unwrap();
        accept(
            &transport,
            proto::server_message::Body::Frame(proto::Frame {
                sequence: 2,
                body: Some(proto::frame::Body::Gap(proto::Gap {
                    first_sample: missing + 1,
                    sample_count: 10,
                    reason: "overflow".into(),
                })),
            }),
        )?;
        accept(
            &transport,
            proto::server_message::Body::End(proto::End {
                next_sequence: 3,
                next_sample: missing + 11,
                tags: vec![],
            }),
        )?;
        source.work()?;
        assert_eq!(handle.lost_samples(), missing + 10);
        assert_eq!(handle.trailing_gap().unwrap().sample_count, 10);
        assert!(output.eof());
        Ok(())
    }
    #[tokio::test]
    async fn rejects_frames_before_credit_and_loss_in_lossless_mode() -> Result<()> {
        let options = StreamOptions::default();
        let (_source, _output, _handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        assert!(accept(&transport, chunk(0, 0, &[1.0], vec![])).is_err());
        transport.next_control().await.unwrap();
        assert!(
            accept(
                &transport,
                proto::server_message::Body::Frame(proto::Frame {
                    sequence: 0,
                    body: Some(proto::frame::Body::Gap(proto::Gap {
                        first_sample: 0,
                        sample_count: 1,
                        reason: String::new()
                    }))
                })
            )
            .is_err()
        );
        assert!(accept(&transport, chunk(1, 0, &[1.0], vec![])).is_err());
        assert!(transport.finish().is_err());
        Ok(())
    }
    #[tokio::test]
    async fn full_output_preserves_frame_and_withholds_credit() -> Result<()> {
        let mut options = StreamOptions::default();
        options.limits.max_in_flight_frames = 1;
        let (mut source, output, _handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        let mut buffer = source.dst.as_ref().unwrap().write_buf()?;
        let len = buffer.len();
        buffer.slice().fill(0.0);
        buffer.produce(len, &[]);
        transport.next_control().await.unwrap();
        accept(&transport, chunk(0, 0, &[2.0], vec![]))?;
        assert!(matches!(source.work()?, BlockRet::WaitForStream(_, 1)));
        assert_eq!(transport.handle.lock().retired, 0);
        let (buffer, _) = output.read_buf()?;
        buffer.consume(len);
        source.work()?;
        let (buffer, _) = output.read_buf()?;
        assert_eq!(buffer.slice(), &[2.0]);
        assert_eq!(transport.handle.lock().retired, 1);
        Ok(())
    }
    #[tokio::test]
    async fn closing_graph_consumer_cancels_an_idle_source() -> Result<()> {
        let options = StreamOptions::default();
        let (mut source, output, handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        drop(output);
        assert!(matches!(source.work()?, BlockRet::EOF));
        assert_eq!(handle.status(), SourceStatus::Cancelled);
        assert!(matches!(
            transport.next_control().await.unwrap().body,
            Some(proto::client_message::Body::Cancel(_))
        ));
        assert!(transport.next_control().await.is_none());
        Ok(())
    }
    #[tokio::test]
    async fn failure_after_complete_cannot_become_success_on_clean_close() -> Result<()> {
        let options = StreamOptions::default();
        let (mut source, _out, handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        transport.next_control().await.unwrap();
        accept(
            &transport,
            proto::server_message::Body::End(proto::End::default()),
        )?;
        source.work()?;
        let complete = match transport.next_control().await.unwrap().body.unwrap() {
            proto::client_message::Body::Complete(c) => c,
            _ => panic!("expected Complete"),
        };
        accept(&transport, proto::server_message::Body::Complete(complete))?;
        transport.fail("peer failed after Complete");
        assert!(transport.finish().is_err());
        assert!(matches!(handle.status(), SourceStatus::Failed(_)));
        assert!(source.work().is_err());
        Ok(())
    }
    #[tokio::test]
    async fn credits_saturate_without_wrapping_at_uint64_limit() -> Result<()> {
        let options = StreamOptions::default();
        let (_source, _out, _handle, transport) =
            IqStreamSource::<f32>::from_started(started::<f32>(&options), &options)?;
        {
            let mut state = transport.handle.lock();
            state.retired = u64::MAX - 1;
            state.next_sequence = u64::MAX;
            state.grant = u64::MAX - 2;
        }
        let message = transport.next_control().await.unwrap();
        assert!(matches!(
            message.body,
            Some(proto::client_message::Body::FlowControl(
                proto::FlowControl {
                    send_limit: u64::MAX
                }
            ))
        ));
        Ok(())
    }
    #[test]
    fn rejects_format_rate_and_metadata_changes_at_opening() {
        let options = StreamOptions::default();
        let mut bad = started::<crate::Complex>(&options);
        assert!(IqStreamSource::<f32>::from_started(bad.clone(), &options).is_err());
        bad.description.as_mut().unwrap().encoding = Some(f32::encoding());
        bad.description.as_mut().unwrap().sample_rate_hz = f64::NAN;
        assert!(IqStreamSource::<f32>::from_started(bad.clone(), &options).is_err());
        bad.description.as_mut().unwrap().sample_rate_hz = 1.0;
        bad.description.as_mut().unwrap().uses_terminal_tags = true;
        assert!(IqStreamSource::<f32>::from_started(bad, &options).is_err());
    }
}
