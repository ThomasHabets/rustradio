# Data stream protocol

This is a small framed protocol for carrying one or more named byte streams over
a bidirectional byte transport such as stdin/stdout or a websocket.

The implementation lives in `src/data_stream.rs`. It exposes a synchronous API
with `SyncReader` and `SyncWriter`, and an asynchronous API with `AsyncReader`
and `AsyncWriter` behind the `async` feature.

## Framing

All integers are little-endian.

Each packet is:

```text
u32 packet_len
u8  packet_type
u8[packet_len - 1] packet_body
```

`packet_len` is the number of bytes after the length field, including the packet
type byte. A length of zero is invalid. Packet type zero is invalid.

The library reader defaults to rejecting packet payloads larger than 64 MiB.

## Stream identifiers

Stream identifiers are represented by the `DataStreamId` newtype. On the wire,
they are UTF-8 bytes. Invalid UTF-8 is rejected.

## Packet types

### Version: type 1

Both sides must send a Version packet before any other packet. Version 0 is the
current protocol version.

```text
u32 packet_len = 5
u8  packet_type = 1
u32 version = 0
```

Readers validate this with `read_version()`.

### RequestData: type 2

Data is pulled by sending RequestData. The receiver sends the current byte
window it is prepared to accept for a stream.

```text
u32 packet_len = 1 + 4 + stream_id_len
u8  packet_type = 2
u32 window
u8[stream_id_len] stream_id
```

The stream ID is the rest of the packet body after `window`.

RequestData may be sent again at any time. The sender replaces the previous
window for that stream with the new value. A zero window tells the sender to
stop sending that stream until a later non-zero RequestData arrives.

### Data: type 3

Data carries bytes for one stream.

```text
u32 packet_len = 1 + 4 + stream_id_len + data_len
u8  packet_type = 3
u32 stream_id_len
u8[stream_id_len] stream_id
u8[data_len] data
```

The sender must not send more bytes than the current requested window for that
stream. After sending a Data packet, the sender reduces its local window for
that stream by `data_len`.

## Status

Implemented:

* Version
* RequestData
* Data
* Sync reader/writer API
* Async reader/writer API

Not implemented yet:

* Message/control packet types for actions such as changing frequency
* In-protocol stream metadata
