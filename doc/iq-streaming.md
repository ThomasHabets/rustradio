# Real and complex sample streaming over gRPC and WebSocket

Status: proposed protocol version 1. The schema is
[iq_stream.proto](../proto/iq_stream.proto). This document specifies the wire
contract; it does not add a transport implementation or dependencies.

## Goals and model

Connect native RustRadio programs and a WASM frontend to a native backend.
Carry sample data and tags together, support continuous reception and finite
file transfers, and leave room for SigMF adapters and GNU Radio source/sink
blocks.

One session carries **one real or complex channel in one direction**. Upload
means client to server; download means server to client. The client initiates
either operation. Two sessions are required for two opposing sample streams.
A connection can remain open indefinitely, but sample encoding and sample rate
are fixed after the handshake. Changing either requires a new session.

The gRPC method is bidirectional because acknowledgements and backpressure
travel opposite the samples. It carries samples in one direction. Ordered
messages within each direction are a gRPC guarantee; the application rules
below define ordering across those directions.
[gRPC core concepts](https://grpc.io/docs/what-is-grpc/core-concepts/)

Nonempty resource identifiers select a server-managed source or destination,
such as a graph port or recording. They are opaque identifiers, not remote
filesystem paths. Discovery, graph configuration, tuning, and creation of
recording destinations belong to a separate control API.

### Flow control and scope

The protocol uses a frame-count window for simple credit accounting. At most
`window * max_frame_bytes` encoded frame data can be outstanding per session;
small chunks use less of that budget but each consumes one slot. The frame
sequence counts storage slots, while the sample cursor counts samples including
gaps.

Version 1 supports one channel per session and bounded metadata per message.
It does not define seeking, resume, a transaction identifier for querying
completion after a disconnect, or metadata added retrospectively to already
sent samples. Separate sessions do not provide atomic synchronization between
channels.

## Transport and opening a stream

### Native gRPC

Use `rustradio.iq.v1.IqStreaming/Stream` over HTTP/2. Each protobuf envelope is
one gRPC message. Use TLS for remote connections; credentials travel in gRPC
metadata. Stream sample metadata lives in protobuf messages, not HTTP headers.

### Browser/WASM

Use a binary WebSocket endpoint at `/iq/v1/stream` with subprotocol
`rustradio.iq.v1`. One complete binary WebSocket message contains exactly one
serialized `ClientMessage` or `ServerMessage`, according to direction. There is
no base64, protobuf JSON, extra length prefix, or gRPC framing inside it.
WebSocket fragmentation is reassembled before protobuf decoding. Text messages
are invalid. One WebSocket connection carries one session and closes when it
ends; there is no multiplexing or reuse for another Open on that connection.

The endpoint adapts these envelopes to the same session handler, or forwards
them to the native gRPC method. This is a specified adapter, rather than an
assumption that ordinary gRPC runs over WebSocket. The official gRPC-Web client
currently supports unary and server-streaming calls, but not client or
bidirectional streaming; it cannot provide this upload path.
[gRPC-Web streaming support](https://github.com/grpc/grpc-web#streaming-support)

Use `wss` remotely. Authenticate at upgrade using the deployment's cookie or
short-lived credential mechanism, and check browser origins. Keep credentials
out of source/destination identifiers. Application credits are essential:
browser receive queues must not grow without a bound.

`WebSocket.send()` queues data; use `bufferedAmount` to enforce a local bound on
unsent bytes in addition to application credit. Reserve room for the small
control messages. Keep receive/control handling active while a sink stalls and
bound decoded messages waiting for WASM or graph processing. The sender also
needs a bounded acquisition queue; receiver credit alone does not bound it.
[WebSocket send and buffering](https://websockets.spec.whatwg.org/#dom-websocket-send)

Disable gRPC message compression and WebSocket per-message compression by
default for sample frames. Compression is a deployment choice after measuring
its CPU cost and bandwidth benefit; limits always apply to uncompressed
protobuf messages, with transport limits also bounding compressed input.
[gRPC compression](https://grpc.io/docs/guides/compression/)

### Handshake

The first client message is exactly one `Open`, containing version `1`, positive
limits, an explicit loss policy and completion mode, and one operation:

- **Upload:** the client supplies the destination and complete description.
  The server accepts that description unchanged or rejects the request.
- **Download:** the client supplies the source, supported encodings, tag kinds
  and opaque codecs, and whether it can preserve terminal tags and tag source
  IDs. The server selects an encoding and a compatible source description.
  Conversion is allowed only to an explicitly offered encoding, with meaning
  documented by the source adapter; offering formats does not request
  retuning/resampling.

The download encoding list must be nonempty. Encoding and tag-kind lists are
sets: duplicates and UNSPECIFIED enum values are invalid. Required message
fields must be present, even though proto3 does not enforce this in the schema.
Opaque codec lists are sets of nonempty, exact, versioned identifiers; wildcards
are invalid. A source's declared codecs must all be accepted by its destination.
`uses_tag_source_ids` requires the download client's `accept_tag_source_ids`,
or equivalent upload-sink support. Source declarations may be conservative
supersets of what is eventually emitted; an unsupported declaration is rejected
without stripping metadata to make the session compatible.

The server replies with exactly one `Started` or a terminal failure.
`Started` contains the immutable description, selected limits, and the exact
requested loss policy and completion mode. Limits may be reduced, never raised;
unsupported policies/modes are rejected, never silently weakened.
For upload, compare descriptions by known field values and optional presence,
not serialized protobuf bytes. Capability lists compare as sets; properties
compare by key and typed value, while sequences/dictionaries keep their order.
Compare floating metadata by value with same-kind NaNs equal and signed zeros
equal; sample payload bits remain untouched. This lets an unchanged description
containing NaN metadata match its echo. Validate a supplied sample_zero_time
against the standard Timestamp bounds.

For upload, `Started.initial_credit` grants the first send window. For download,
this field is absent: the client validates `Started`, then sends its initial
`FlowControl`. That message both accepts the description and grants credit.
Neither frames nor `End` may be sent before this negotiation completes.

```text
Upload:   client Open(description) -> server Started(credit)
          client Frame* / End      -> server FlowControl* / Complete

Download: client Open(capabilities)-> server Started(description)
          client FlowControl      -> server Frame* / End
          client FlowControl* / Complete -> server Complete
```

`*` means zero or more messages; messages in opposite directions can overlap.
Any message in the wrong phase or sample direction is a protocol error. A second
`Open`/`Started`, missing message body, or unsupported version is also an error.
Validate resource compatibility and description sizes before replying `Started`.
No source needs to start acquisition merely to wait for the initial credit.
An already running live source must establish the session's sample origin and
apply its loss policy to any losses after that origin, including while opening.

| Phase | Upload | Download |
| --- | --- | --- |
| Opening | Client Open; server Started with initial credit | Client Open; server Started; client initial credit |
| Sending | Client Frame; server credit | Server Frame; client credit |
| Finishing | Client End; server accepts/finalizes destination and sends Complete | Server End; client accepts/finalizes destination and sends Complete; server echoes Complete |
| Terminal | Server completes transport; no further messages | Server completes transport; no further messages |

The client may send Failure after Open and before its End/Complete. Cancel can
also interrupt upload finalization after End, if the send side remains open.
Neither follows client Complete or a terminal server response. Native RPC
cancellation can interrupt a wait after half-close, and closing a WebSocket
interrupts its session. Cancellation may race completion and cannot undo a
finished file. The server may fail at any phase. Credits already in transit
may arrive after End. Stop issuing credits once End is received and serialize
a download client's final Complete after any queued credits; nothing follows
Complete on that send side.
Use one ordered writer per direction so terminal messages cannot be overtaken
by queued data/credits. Commit each outgoing phase/grant transition before
awaiting its transport write: the peer can receive Started, a credit, End, or
Complete and reply before that write finishes locally. Record submitted frame
counts at the same point. If the write fails, fail the session instead of
rolling back state and retrying a message.

On cancellation/failure, stop producing messages and discard queued sends;
opposite-direction messages already in transit can still arrive before the
server's terminal response. After sending Cancel or Failure, the client may
half-close and continue receiving the terminal response. Without Cancel/Failure,
an upload half-close before End, or a download-client half-close before Complete,
is a truncated transfer. Standard protobuf oneof/merge rules apply when decoding;
the resulting message must have a recognized body. There is no requirement to
implement a separate canonical protobuf encoding.
[Protobuf oneof parsing](https://protobuf.dev/programming-guides/proto3/#oneof)

## Samples and stream positions

The description fixes the component type, byte order, sample layout, and finite,
positive `sample_rate_hz`. `Encoding.layout` must explicitly be `REAL` or
`COMPLEX`; an omitted or `UNSPECIFIED` layout is invalid. Samples have no padding:

- **REAL:** packed `x[0], x[1], ...`. One sample is one scalar component.
- **COMPLEX:** packed `I[0], Q[0], I[1], Q[1], ...`. One sample is one I/Q pair.

Sample indices, counts, tag positions, gaps, and sample rate all refer to these
complete samples. Multiple channels use separate sessions; there is no implicit
channel interleaving. Each accepted encoding is an exact
`(component_type, byte_order, layout)` combination. Changing layout requires a
new session. Layout conversion must be explicitly defined by the source adapter;
offering both layouts does not request discarding Q or synthesizing a zero Q.

| Component type | Bytes per real sample | Bytes per complex sample | Representation |
| --- | ---: | ---: | --- |
| FLOAT32 | 4 | 8 | IEEE 754 binary32 |
| FLOAT64 | 8 | 16 | IEEE 754 binary64 |
| INT8 / UINT8 | 1 | 2 | Signed two's complement / unsigned |
| INT16 / UINT16 | 2 | 4 | Signed two's complement / unsigned |
| INT32 / UINT32 | 4 | 8 | Signed two's complement / unsigned |
| INT64 / UINT64 | 8 | 16 | Signed two's complement / unsigned |

Multibyte components require explicit little or big endian. Eight-bit components
require `NOT_APPLICABLE`. Transport implementations should support
FLOAT32/little-endian with both REAL and COMPLEX layouts, matching RustRadio's
`Float` and `Complex` sample types respectively. Each resource advertises only
the combinations it can produce or consume; a real graph input does not have
to accept complex samples. All formats are negotiated. Raw samples have no
implicit scaling or normalization. An adapter
that converts integer counts to floating point must explicitly document its
scaling; the protocol itself makes no numerical conversion. Sample bytes,
including IEEE exceptional values and their payload bits, are preserved when
the encoding is unchanged.

Each session starts with frame sequence `0` and sample cursor `0`. A `Frame`
contains one `SampleChunk` or one `Gap`; its sequence must be the next expected
number. For a chunk:

1. `first_sample` must equal the sample cursor.
2. `sample_count` must be positive.
3. `samples.len()` must equal `sample_count * bytes_per_sample`, checked without
   integer overflow.
4. Tags must satisfy the rules below.
5. Advance the cursor by `sample_count` and frame sequence by one.

Reject duplicates, out-of-order frames, undeclared jumps, truncated samples, and
counter overflow. These are ordered transports, so there is no reorder buffer,
per-chunk checksum, retransmission, or automatic reconnect/resume in version 1.
A new connection is a new session; callers must explicitly decide whether a
file operation can be restarted without duplicating side effects.

`source_sample_offset`, if present, locates sample zero in the original source
dataset; wire positions still start at zero. `sample_zero_time`, if known, is a
nominal UTC anchor. At fixed rate, sample `n` nominally occurs at
`sample_zero_time + n / sample_rate_hz`. It is not a promise of clock accuracy;
capture/time tags can supply more accurate anchors. These optional fields do
not change cursor validation.

Example: a FLOAT32/LE chunk starting at sample 4096 with 1024 samples has 4096
sample bytes with REAL layout or 8192 bytes with COMPLEX layout. Either can
contain tags at indices 4096 through 5119; the next chunk or gap starts at 5120.
Frames carry `samples` bytes and inherit their layout from the description;
there is no per-frame format override.

## Tags and metadata

Tags travel in the same chunk as their associated samples. Each carries an
absolute `sample_index`, a nonempty UTF-8 key, a typed value, and optional source
provenance. The index must lie in the chunk's half-open sample range. Tags are
sorted by index; duplicates and original order at the same index are preserved.
Use a repeated field, not a map. There is no late insertion into an earlier
chunk and no separate tag stream to synchronize.

The basic required value kinds are FLOAT32, BOOL, INT64, UINT64, and STRING,
covering current RustRadio `TagValue`. Other kinds support metadata and future
GNU Radio adapters: FLOAT64, BYTES, SYMBOL, COMPLEX, LIST, TUPLE, PAIR,
DICTIONARY, NIL, JSON, and OPAQUE. A description declares every kind it might
use, including nested kinds and property values. Download capabilities must
cover this declaration; an upload receiver checks it before accepting. Empty
kind lists are valid only when no properties or tags will be sent.

Every `TagValue` must have its oneof set: false, zero, empty strings/bytes and
NIL remain distinct. Pair members and dictionary entries require present values;
dictionary keys are restricted to BOOL, INT64, UINT64, STRING, SYMBOL, BYTES,
or NIL, and are unique by kind and exact content. This keeps key equality
portable; an adapter rejects other dictionary keys explicitly. JSON is one
valid JSON value retained as text, so integers need not pass through a double.
Its parsers must also enforce nesting and allocation limits.
Opaque data needs a nonempty, versioned codec `type_url`, listed in
`description.opaque_codecs`; a nonempty list requires TAG_KIND_OPAQUE. The
download receiver lists exact `accepted_opaque_codecs`, and an upload sink
checks its own codec support. An empty list prohibits opaque values, including
nested values and properties. Listing a codec may promise storage/forwarding
without interpretation, but that behavior must be explicit in the adapter.
Never fetch the identifier as a URL. An undeclared codec is a protocol error.
No native Rust or PMT object memory image is a portable encoding.

Unknown, unset, undeclared, or unsupported value kinds fail the stream. Do not
silently remove tags or round 64-bit integers through JavaScript `Number`.
WASM/JavaScript bindings must retain protobuf uint64/int64 as integers, BigInt,
or an equivalent lossless representation. A current RustRadio adapter must
reject unsupported values or use a separately specified metadata path; it must
not silently narrow FLOAT64 or discard structured metadata/provenance. Tags
with `source_id` require `description.uses_tag_source_ids`, even when the ID is
an empty string. This lets a graph reject provenance it cannot preserve before
the samples arrive. `description.source_id` remains informational stream origin
and does not authorize tag provenance.

`End.tags` provides optional terminal metadata at exactly `next_sample`, such
as a zero-length file annotation at EOF. It is not attached to a nonexistent
sample and requires `uses_terminal_tags` in the description. Download receivers
must opt in. A graph adapter unable to carry terminal metadata must reject such
a description. This also permits metadata for an empty recording.

Validate the entire frame before publishing samples or tags. An adapter may
then feed a chunk into smaller local buffer windows, adjusting tag positions for
each window; acknowledge it only after the entire chunk is accepted by the
destination. Release its frame slot once the frame storage is also free.
Stream properties are unique keyed, immutable descriptive values; they cannot
override the explicit format or rate fields.

## Backpressure, limits, and gaps

gRPC transport flow control does not by itself specify when a sink has consumed
data; a successful write may only have queued it. Use the same application
credit rules on both transports, in addition to their transport flow control.
[gRPC flow control](https://grpc.io/docs/guides/flow-control/)

`FlowControl` is issued by the sample receiver and contains only `send_limit`,
the exclusive frame sequence it permits. Let `W = max_in_flight_frames`.
The initial grant is exactly `W`. Retire frames in sequence order: after a frame
has been accepted by the destination and its receiver frame storage released,
one slot may be returned by increasing the limit by one. Returning several
slots in one update is encouraged; updates may be delayed to apply backpressure.
Sender validation uses checked arithmetic:

```text
previous_send_limit <= send_limit
W <= send_limit
send_limit - W <= submitted_frames
send frame only if sequence < send_limit
```

For `W = 8`, a grant of `8` permits frames 0 through 7. Releasing frames 0, 1,
and 2 permits a grant of `11`, allowing frames through 10. `send_limit - W` is
the exclusive boundary of returned slots; a separate acknowledgement counter
would repeat it. This conveys neither completed downstream DSP nor durability.
`Complete` is the destination's final acknowledgement.

A sender records a frame as submitted before handing it to the transport:
receiver acknowledgements can race completion of its write operation.
A receiver cannot return slots for frames it has not accepted and released or
retract grants; a sender cannot exceed the last received grant. Gaps spend one
frame credit, just like chunks. End, completion, cancellation, failure, and
credit messages spend no credit, avoiding a deadlock at the end of a full window.
Reserve bounded receive storage for End in addition to the frame window; its
metadata can arrive while that window is full. Keep all counters representable.
When `send_limit` reaches uint64 maximum, issue no more grants and end after the
remaining permitted frames. End's exclusive cursors must also remain valid.

Recommended initial limits are 256 KiB per encoded `Frame` and eight outstanding
frames, roughly 2 MiB of framed data plus bounded transport/copy overhead. The
version-1 ceilings are 1 MiB per frame and 64 outstanding frames. Configure gRPC
and WebSocket envelope limits consistently: frames have additional envelope
overhead, with a 2 MiB absolute envelope cap. Control envelopes, including
`Open`, `Started`, and `End`, have a 64 KiB cap. Limit nested tag values to depth
32 (counting the outer TagValue as level 1), tags to 4096 per chunk/End, and total
TagValue nodes to 16384 per envelope, including dictionary keys and values and
description properties. Configure decoder recursion/allocation budgets before
decoding, allowing for the message wrappers around TagValues. Encoded byte caps
do not alone bound the size of the decoded object tree. Compare the received
wire byte length to limits; reserialized protobuf size is not authoritative.
A single sample with too many associated tags to fit cannot be split from its
metadata; fail explicitly. Metadata fragmentation is outside version 1: large
SigMF global objects or EOF annotations can exceed these limits and must be
rejected, preferably during opening when the source already knows their sizes.

For live data, flush a partial chunk after about 10 ms rather than waiting for
the maximum frame size. For file transfers, fill larger chunks without pacing
at the sample rate. Send cumulative credits at half-window progress or within
10 ms, whichever comes first; a one-frame window returns its slot promptly.
Coalesce pending credit updates to their largest limit instead of queueing an
unbounded list. Reader/control tasks continue running while sample writers wait
for credit; queued sample data must not prevent control messages from being
serviced. A slow destination stalls credit, rather than
moving unbounded queues into the browser, bridge, or graph. Size the window for
the bandwidth-delay product when latency is higher. The 64-frame/1-MiB ceilings
can limit throughput on paths whose bandwidth-delay product exceeds 64 MiB.
Gap mode does not promise a fresh display: at most a window of already submitted
old frames can still arrive, plus any bounded local acquisition backlog.

A WebSocket-to-gRPC bridge must propagate backpressure across both hops. It may
forward the negotiated credits unchanged while strictly bounding transport
buffers, or terminate each hop's window with its own bounded queue. In the
latter case it returns a slot only after forwarding the frame into bounded
next-hop storage and releasing its own copy. Keep frame boundaries and all
sequence/sample counters unchanged. Relay completion only after the actual
destination's acknowledgement. It cannot complete a recording
merely because an intermediate queue accepted its bytes.

- **LOSSLESS:** pause a pausable producer. If acquisition cannot pause and
  overruns, fail with `DATA_LOSS`; never claim a complete recording.
- **ALLOW_GAPS:** a nonpausable producer may drop unsent samples under pressure.
  Emit a `Gap` before the next retained chunk, with an exact positive lost sample
  count. Associated tags may also have been lost. The gap advances the sample
  cursor, so indices and nominal time do not silently compress. Coalesce adjacent
  losses while awaiting credit; never silently drop already transmitted frames.
  A pending final gap must precede End even when no retained chunk follows it.
  If a lost range may include persistent state such as frequency tags, the
  adapter must re-emit known current state with the next retained sample or mark
  it unknown through its documented gap convention. Never assume tag state
  survived a gap.

Unknown loss counts fail even in gap mode: invented sample indices are worse
than a reported failure. Receivers must make gaps visible to the application;
no automatic zero filling or silent removal. A gap in LOSSLESS mode is an error.
The agreed policies support live display with explicit gaps and lossless file
transfer on the same protocol.

## Ending, acknowledgement, and errors

The source sends exactly one `End` after its last frame. Its `next_sequence`
and `next_sample` must equal the receiver's resulting cursors. No further frames
or `End` are permitted; outstanding credits may still cross in transit. An empty
stream is `End(0, 0)` after a normal handshake.

- **Upload:** the client sends `End` and may half-close its gRPC send side. The
  server accepts all pending data, handles terminal metadata, sends `Complete`
  with matching cursors/mode, then finishes the RPC with OK.
- **Download:** the server sends `End`. After accepting all frames and terminal
  metadata, the client sends matching `Complete` and may half-close. The server
  echoes `Complete` and finishes with OK. This establishes receiver completion;
  merely writing all data into a socket is insufficient.

ACCEPTED means the requested destination accepted the data and metadata,
possibly into its bounded graph buffers. It does not prove subsequent graph
processing or durable storage. For a file, ACCEPTED still requires successful
dataset/metadata finalization and close; any failure there prevents Complete.
DURABLE is only offered by file destinations: the receiver synchronizes the
data, metadata, and final publication before sending its acknowledgement.
Upload/download adapters must implement this mode explicitly; credit
acknowledgements never promise durability. DURABLE applies
to the receiving file in either direction. A download client verifies its local
sink can provide this promise before requesting it; it does not require a
durable source. Graph sinks must also accept the end-of-input notification after
all data, though downstream DSP may still be pending.

Success requires the final server `Complete` and successful transport completion:
gRPC OK, or a normal WebSocket close after `Complete`. The server ends the
WebSocket session with close code 1000 on success. Clients must keep receiving
until then. An early EOF, close, disconnect, or missing acknowledgement is
not an observed success, even if some samples arrived. Before End the transfer
is incomplete; after End a disconnect can leave the outcome unknown because
the destination may have finalized the recording while its acknowledgement was
lost. The protocol provides no exactly-once retry or status query. Cancellation
does not roll back already accepted samples or file writes. Do not automatically
retry a partially completed upload.

Either peer may cancel; the client can send `Cancel` or use native RPC
cancellation. A client source/sink error uses `ClientMessage.failure`, for
example when a download cannot finish writing its local file. The server stops
the source and returns terminal Failure with the corresponding status; it may
instead report its own error if both peers fail concurrently. After sending
Failure, a peer sends no further application messages. The server reports a
terminal `Failure` where possible. Its code matches the gRPC status; native
trailers remain authoritative when a transport failure prevents a message.
A final Complete followed by a non-OK status fails;
a Failure followed by OK is a protocol error. WebSocket sends `Failure` then
closes (1002 for protocol errors, 1011 for server failures). Unexpected close
is always failure.

| Condition | gRPC status |
| --- | --- |
| Malformed envelope, invalid counters/data/tags/limits | INVALID_ARGUMENT |
| Unsupported version, encoding, tag kind, or completion mode | UNIMPLEMENTED |
| Unsupported declared opaque codec, provenance, or terminal metadata | UNIMPLEMENTED |
| Unknown resource | NOT_FOUND |
| Incompatible source/destination configuration | FAILED_PRECONDITION |
| Frame/window resource limit exceeded | RESOURCE_EXHAUSTED |
| Source overflow in lossless mode, truncated transfer | DATA_LOSS |
| Authentication/authorization failure | UNAUTHENTICATED / PERMISSION_DENIED |
| Cancellation / expired deadline | CANCELLED / DEADLINE_EXCEEDED |
| Source/destination I/O failure | UNAVAILABLE or INTERNAL, as appropriate |

Use deadlines for the handshake and finite transfers. Continuous streams may
have no overall deadline; deployment-specific liveness timeouts/keepalives are
separate from sample rate and credit. Suspend acquisition or bound loss counters
while a receiver is unavailable; connection loss ends the session.

## Adapter profiles

### RustRadio

Keep network I/O outside the block scheduler's synchronous work function, using
bounded queues and asynchronous tasks. Source/sink blocks translate between
absolute wire indices and local buffer-relative tags. Use batch serialization
and the existing little-endian `Float` or interleaved `Complex` layout, matching
the graph port's sample type to the negotiated layout. Reuse chunk buffers and
avoid per-sample protobuf messages. A copied protobuf `bytes` buffer is acceptable;
do not promise end-to-end zero-copy, particularly with WASM linear snapshots.

The experimental implementation is gated by `unstable` in both `rustradio` and
`rustradio-ui`. The native `IqServer` registry serves raw gRPC and the WebSocket
endpoint on one listener. Several resource IDs may be served concurrently; a
resource admits one active client, with `ALREADY_EXISTS` for a second client.
The same `IqStreamSink<T>` serves both transports. `IqStreamSource<T>` is a native
gRPC client; `rustradio_ui::worker::IqStreamSource<T>` is a browser WebSocket
client. The initial adapters support `Float` and `Complex` in little-endian
FLOAT32 layout, the five scalar RustRadio tag kinds, and ACCEPTED completion.
Uploads and DURABLE completion return `UNIMPLEMENTED`. Structured values,
provenance and terminal tags are not accepted by graph sources.

A sink's `.blocking(true)` setting preserves input when its bounded queue fills
and waits while disconnected. `.blocking(false)` consumes and discards newly
arriving samples under pressure; it coalesces exact losses into a Gap while
preserving queued frames. Loss also discards associated tags. With no connected
client, nonblocking sinks discard input without retaining history. Each
connection starts fresh at the current graph position, advertised through
`source_sample_offset`; wire counters restart at zero. A disconnected session
has failed, and its pending queues are discarded. A later session cannot replay
what that client missed. EOF without an active client closes the resource.

Sources default to LOSSLESS. Set `StreamOptions.loss_policy` to ALLOW_GAPS to
connect to a nonblocking sink; the requested policy must match the sink setting.
A received gap is aggregated into bounded pending state. At the first retained
sample afterward, the source attaches two U64 tags, in this order:

- `rustradio.iq.gap_samples`: total missing samples since the last retained range.
- `rustradio.iq.sample_index`: that sample's absolute session index, without
  adding `source_sample_offset`.

These keys are reserved and rejected as ordinary wire or sink-input tags. The
markers mean persistent tag state is unknown; downstream DSP must handle the
discontinuity explicitly. Retained samples occupy consecutive local buffer
positions, with the markers preserving the lost timeline. No zeros are inserted.
A SourceHandle exposes negotiated metadata, status, cumulative loss, End cursors,
and a trailing gap, including loss immediately before EOF. Adjacent gap reasons
may be replaced with a general diagnostic when aggregated; exact counts remain.
A gap consumes and releases a frame slot when folded into this bounded state,
so consecutive gaps cannot deadlock a one-frame window.

Native networking runs on the application's Tokio runtime. Run a synchronous
Graph with `spawn_blocking`, or use MTGraph or AsyncGraph. Completed graph blocks
release their streams so sinks can observe EOF. A graph source closes its output
once all frames and End have been accepted, then waits for the server Complete
and successful transport termination before returning EOF. ACCEPTED does not
wait for downstream DSP to finish. Dropping a source cancels its connection;
source failures require a new source/graph, with no automatic reconnect. Server
shutdown fails registered graph sinks and aborts sessions; applications mounting
the router themselves call `IqServer::shutdown` alongside HTTP shutdown.

The browser source keeps socket handles in worker-local tasks and accepts the
wake sender paired with `WasmGraph::run_async`. All network callbacks notify that
sender. Ordinary graph backpressure withholds credits. Native listeners expose
an Axum router for application integration and TLS termination; browser clients
use `wss://` for TLS. Native gRPC clients accept `https://` with system trust
roots. The standalone listener serves plaintext HTTP locally.

For a native roundtrip, run:

```sh
cargo run --features unstable --example iq_stream
```

A browser worker can connect as follows (the graph's processing blocks follow
`output`):

```rust,ignore
use rustradio::{Complex, graph::GraphRunner};
use rustradio::iq_stream::StreamOptions;
use rustradio_ui::worker::IqStreamSource;

let (poke, wake) = async_channel::bounded(1);
let (source, output, handle) = IqStreamSource::<Complex>::connect(
    "ws://localhost:50051/iq/v1/stream", "iq", StreamOptions::default(), poke,
).await?;
let rate = handle.description().sample_rate_hz;
let mut graph = rustradio::wasm::wasm_graph::WasmGraph::new();
graph.add(Box::new(source));
// Add DSP and a sink connected to output, using rate for configuration.
graph.run_async(wake).await?;
```

Generated prost messages and the decoder-budget schema are checked in. Regenerate
both with `/usr/bin/python3 tools/generate_iq_proto.py` (requires `protoc` and
Python's protobuf package). Normal builds need neither tool. Semver checks must
exclude `unstable`; enabling every feature includes these experimental APIs.

### SigMF file source and sink

The transport is independent of the file container. A future file source uses
its dataset encoding and sample rate to describe the stream; a sink can write
chunk bytes directly when the dataset encoding matches. Otherwise the adapter
performs an explicit conversion and updates the recorded datatype. The adapter
must preserve the dataset's real/complex distinction in `Encoding.layout`.
File resources accept only supported SigMF datatypes and metadata ranges;
INT64/UINT64 samples have no SigMF core datatype and require a specified extension
or rejection. Check this compatibility during opening.
[SigMF dataset format](https://sigmf.org/)

Define this metadata profile:

- Property `sigmf:global` is a JSON object containing the complete global
  metadata. Its datatype/rate/channel count must agree with the description;
  absent rate must be supplied explicitly by the caller. Transport supports
  exactly one real or complex channel. Preserve extension fields; never reuse
  a dataset hash after sample conversion, cropping, or other changes.
- Tags `sigmf:capture` and `sigmf:annotation` hold each complete JSON object.
  These keys require JSON record values in this file profile. Remove
  `core:sample_start` from the object and map it to the wire index as below;
  reconstruct it when writing. Retain annotation `core:sample_count` and all
  extension fields in the JSON value, including annotations extending across
  later chunks.
- Emit records at their start position, preserving their order within each file
  array. At equal positions, order unmarked captures before unmarked annotations;
  records with the extension's order markers follow the rule below.
  Records at EOF use terminal tags. For a selected recording segment,
  rebase indices to zero and retain source origin; annotations/captures affected
  by cropping need explicit normalization by the file adapter.

SigMF positions include `core:offset` (default zero). Let `B` be that offset and
`S` the selected segment's starting sample within the dataset. Wire zero maps
to `O = B + S`: set `source_sample_offset = O`, and convert a record at SigMF
position `P` to wire index `P - O`. Normalize records affected by cropping first;
reject other records outside the streamed range instead of wrapping subtraction.
For output, use dataset origin `D = source_sample_offset` when present, otherwise
zero. Set `core:offset = D` and record positions to `D + wire_index`, including
indices inside stored protobuf Tags. Rebase them on replay. Use checked integer
arithmetic and validate stored indices/counts against SigMF's bounds. For example,
a dataset starting at 1000 with a record at 1005 carries that record at wire
index 5, and restores it to 1005 in a file with the same origin.
[SigMF offsets](https://sigmf.org/#offset)

File sinks write the negotiated datatype, sample rate, and one-channel layout
to the output global metadata, including a rate supplied by the caller when it
was absent from the input file. Normalize dataset filename references to the
actual output recording; incoming metadata cannot choose its filesystem path.

The initial file profile supports conforming datasets only. Dataset headers,
trailing non-sample bytes, or capture `core:header_bytes` need an explicitly
specified normalization that updates file-layout metadata; reject these inputs
until implemented. Streaming packed samples cannot preserve their original file
bytes simply by copying the global/capture JSON.
[SigMF non-conforming datasets](https://sigmf.org/#header_bytes)

Capture and annotation arrays use sample indices, and annotation lengths can
extend beyond the chunk carrying their start. Missing annotation length has
SigMF-specific meaning and must remain missing in the JSON object.
[SigMF metadata specification](https://sigmf.org/)

File adapters initially require LOSSLESS. On success, finalize the dataset and
metadata; DURABLE additionally synchronizes both and their publication. Interrupted
recordings remain marked incomplete. Existing recordings are not overwritten by
an automatic retry. A future gap-aware recorder can create segmented recordings
with explicit mappings, rather than compressing the timeline or silently filling
samples. Recordings without a known sample rate cannot be streamed until the
caller supplies one; format/rate changes require separate sessions/files.
Finite file sources check that the complete expected sample range was read
before End; an early file EOF or a partial final sample fails with DATA_LOSS.

Other stream tags also need storage: propose a versioned `rustradio` SigMF
extension, published as `rustradio.sigmf-ext.md` before implementing the file
adapter. Its `rustradio:tag_proto` annotation field contains base64 of the full
version-1 protobuf `Tag`, including its typed value and provenance. Use
`core:sample_count = 1` for ordinary tags and `0` for terminal tags, with
`core:sample_start` and the encoded index both referring to the file dataset.
Declare extension version `1.0.0` with `optional: false` in `core:extensions`.
On replay, recognize these wrapper annotations, validate their indices, and
restore the original tags instead of also emitting duplicate `sigmf:annotation`
tags. Recognize wrappers only with the declared, supported extension. Wrapper
annotations contain only `core:sample_start`, `core:sample_count`,
`rustradio:tag_proto`, and the required order marker below; reject extra fields
rather than lose them while unwrapping.

The extension also defines `rustradio:tag_order` on captures and annotations:
the zero-based ordinal of the corresponding wire tag among all tags at that
sample index. Writers add it to every record, including protobuf wrappers.
Readers merge the arrays using these ordinals, then remove the marker from
emitted record JSON. For each index, either no records are marked (use the
capture/annotation order above), or all are uniquely numbered `0..n-1`; reject
partial, duplicate, or noncontiguous markers. This preserves stable tag order
across the two arrays. SigMF core gives no order to equal-position annotations.
[SigMF annotation ordering](https://sigmf.org/#annotations-array)

This is a proposed extension, not part of SigMF core. Base64 is confined
to the JSON file; the streaming transport remains binary.
[SigMF extensions](https://sigmf.org/)

The file adapter must retain full JSON metadata, beyond the fields modeled by
the current RustRadio SigMF structs. Adapters must handle or reject required
extensions, rather than presenting an incomplete metadata round trip as lossless.

Downloading into a local SigMF file uses the download receiver's completion
acknowledgement. Uploading a local SigMF file uses the upload destination's
acknowledgement. Neither requires a separate file-transfer protocol. Bulk replay
is unpaced by default; realtime replay is a source-adapter option.

### GNU Radio

Future source/sink blocks map the absolute wire sample index to GNU Radio's
absolute item offset, rebasing by the block's starting item count. Map a tag key
to a PMT symbol and optional `source_id` to tag provenance. GNU Radio exposes
offset, key, value and source ID in its tag API.
[GNU Radio stream tags](https://wiki.gnuradio.org/index.php/Stream_Tags)

Declare `uses_tag_source_ids` when exporting provenance and require receiver
support. The wire represents provenance as a string; PMT source IDs that cannot
be mapped losslessly to this representation must be rejected or handled by a
separately specified adapter convention.

Use typed mappings for PMT booleans, signed/unsigned integers, real/complex
numbers, symbols, lists, tuples, pairs, supported dictionaries, NIL and blobs.
PMT symbols map to SYMBOL and blobs to BYTES. PMT has no corresponding distinct
ordinary string type: STRING-to-symbol conversion must be explicitly configured,
otherwise that graph adapter rejects STRING. This avoids an implicit loss of
the wire's string/symbol distinction.
[GNU Radio PMT API](https://www.gnuradio.org/doc/doxygen/namespacepmt.html)

Adapters must document their conversion of lists/pairs and floating-point kinds.
PMT's empty list maps to NIL, while an empty wire LIST is a distinct kind.
An adapter that promises kind preservation must retain that distinction using
an agreed wrapper or reject the conversion. Numeric widening can preserve a
value without preserving its original FLOAT32/FLOAT64 kind; document the return
mapping too.
[GNU Radio empty-list representation](https://www.gnuradio.org/doc/doxygen/namespacepmt.html)

For example, a receive-time tuple retains its uint64 seconds and
floating fractional seconds. Do not turn all PMTs into strings or doubles.
Uniform vectors/custom types require an agreed opaque codec or rejection;
OPAQUE is not permission to use a version-specific PMT serialization implicitly.
Reject unsupported values before forwarding their associated samples. Standard
tag semantics such as timestamps and center frequency are adapter conventions,
not changes to the session's sample format or rate.

## Compatibility and implementation checks

Version 1 uses proto3 with presence-bearing message/oneof fields. Add optional
fields without reusing field numbers; reserve numbers/names when removing fields.
Unknown descriptive fields can be ignored, but unknown frame bodies/value kinds
are errors. A semantic change to ordering, encoding, credit, or completion rules
requires a new protocol/package version and WebSocket subprotocol. Keep server
limits configurable downward. Implement FLOAT32/LE in both layouts and the basic
tag profile; advertise other capabilities only when the corresponding adapters
exist.

Implementation acceptance tests should cover:

- Upload/download over native gRPC and the binary WebSocket adapter; reject
  unsupported descriptions before samples flow.
- Bit-exact real and complex FLOAT32/FLOAT64 transfers, endian/layout/count
  checks, rejection of missing/unsupported layouts, empty streams, and
  indices/uint64 tags beyond JavaScript's exact-number range.
- Duplicate ordered tags, zero/false/empty values, nested values, terminal tags,
  provenance/codec negotiation, decoder allocation/count/depth limits, and tag
  rebasing through partial local buffer windows.
- Slow receivers, exhausted credit, control-message progress, bounded queues,
  fixed-window credit validation/batching, explicit final gaps, uint64 counter
  exhaustion, bridge backpressure, unknown loss counts, and lossless overrun failure.
- Completion after the actual file sink, durable acknowledgement, cancellation,
  replies before write completion, client sink Failure, half-close after
  Cancel/Failure, End at a full window, disconnect before/after End, ambiguous
  outcomes, missing final acknowledgement, and partial files.
- SigMF round trips with nonzero offsets, stable order across arrays, wrapper
  validation, capture/annotation JSON and extension fields; GNU Radio
  scalar/receive-time tag mappings, list/NIL and float-kind conversions, and
  explicit unsupported-value failures.
- Sustained throughput and latency at realistic sample rates, measuring sample
  copies, protobuf/tag cost, queue memory and credit-window sizing.

Compile the schema without adding a repository dependency:

```sh
protoc -I proto -I /usr/include --include_imports \
  --descriptor_set_out=/tmp/iq_stream.pb proto/iq_stream.proto
```
