//! Experimental real/complex sample streaming. Enable the `unstable` feature.
//!
//! Native graphs use gRPC clients and a shared gRPC/WebSocket server. Browser
//! workers use the WebSocket client in `rustradio-ui`. All queues are bounded;
//! graph `work()` methods never wait on sockets. See `doc/iq-streaming.md`.

#[allow(clippy::large_enum_variant)]
pub mod proto;
mod source;
mod wire;
mod wire_schema;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
mod rpc;
#[cfg(not(target_arch = "wasm32"))]
mod sink;

#[cfg(not(target_arch = "wasm32"))]
pub use native::IqServer;
#[cfg(not(target_arch = "wasm32"))]
pub use sink::{IqStreamSink, IqStreamSinkBuilder};
pub use source::{IqStreamSource, SourceHandle, SourceStatus, SourceTransport, StreamOptions};
pub use wire::{Decoded, decode_client, decode_server};

#[cfg(not(target_arch = "wasm32"))]
use crate::stream::Tag;
use crate::stream::TagValue;
use crate::{Complex, Error, Float, Result, Sample};

/// Gap count attached to the first retained sample after a discontinuity.
pub const GAP_SAMPLES: &str = "rustradio.iq.gap_samples";
/// Session sample index of the first retained sample after a discontinuity.
pub const SAMPLE_INDEX: &str = "rustradio.iq.sample_index";
/// The two reserved tags signal that persistent tag state is unknown after loss.
fn reserved(key: &str) -> bool {
    matches!(key, GAP_SAMPLES | SAMPLE_INDEX)
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for crate::Float {}
    impl Sealed for crate::Complex {}
}

/// Supported packed stream samples. This experimental trait is sealed.
pub trait IqSample: Sample<Type = Self> + sealed::Sealed {
    /// Fixed wire encoding of this graph port.
    fn encoding() -> proto::Encoding;
}
impl IqSample for Float {
    fn encoding() -> proto::Encoding {
        encoding(proto::SampleLayout::Real)
    }
}
impl IqSample for Complex {
    fn encoding() -> proto::Encoding {
        encoding(proto::SampleLayout::Complex)
    }
}
fn encoding(layout: proto::SampleLayout) -> proto::Encoding {
    proto::Encoding {
        component_type: proto::ComponentType::Float32 as i32,
        byte_order: proto::ByteOrder::LittleEndian as i32,
        layout: layout as i32,
    }
}
// Advertise exactly the value types that crate::stream::TagValue can represent.
// The protocol supports more kinds, but accepting them would require a lossless
// graph representation. Protobuf enum fields store their wire values as i32.
const TAG_KINDS: [i32; 5] = [
    proto::TagKind::Float32 as i32,
    proto::TagKind::Bool as i32,
    proto::TagKind::Int64 as i32,
    proto::TagKind::Uint64 as i32,
    proto::TagKind::String as i32,
];
// These are protocol-wide encoded byte limits, applied before allocating the
// decoded message. A Frame has its own smaller, negotiated limit; the envelope
// allowance also covers protobuf fields surrounding it.
const MAX_ENVELOPE: usize = 2 * 1024 * 1024;
const MAX_CONTROL: usize = 64 * 1024;
/// Encode a client control envelope for the binary WebSocket transport.
/// The returned bytes have no gRPC or protobuf length-delimiter prefix.
pub fn encode_client(message: &proto::ClientMessage) -> Result<Vec<u8>> {
    use prost::Message;
    if message.encoded_len() > MAX_CONTROL {
        return Err(err("IQ control byte limit"));
    }
    Ok(message.encode_to_vec())
}
fn err(message: impl Into<String>) -> Error {
    Error::msg(message.into())
}
fn limits_valid(limits: &proto::Limits) -> Result<()> {
    if limits.max_frame_bytes == 0
        || limits.max_frame_bytes > 1024 * 1024
        || limits.max_in_flight_frames == 0
        || limits.max_in_flight_frames > 64
    {
        return Err(err("invalid IQ frame/window limits"));
    }
    Ok(())
}
fn advance(cursor: u64, count: u64) -> Result<u64> {
    cursor
        .checked_add(count)
        .ok_or_else(|| err("IQ stream counter overflow"))
}
#[cfg(not(target_arch = "wasm32"))]
fn tag_to_wire(tag: &Tag, first: u64) -> Result<proto::Tag> {
    use proto::tag_value::Kind;
    if tag.key().is_empty() || reserved(tag.key()) {
        return Err(err("empty or reserved IQ tag key"));
    }
    let kind = match tag.val() {
        TagValue::Float(v) => Kind::Float32Value(*v),
        TagValue::Bool(v) => Kind::BoolValue(*v),
        TagValue::I64(v) => Kind::Int64Value(*v),
        TagValue::U64(v) => Kind::Uint64Value(*v),
        TagValue::String(v) => Kind::StringValue(v.clone()),
    };
    Ok(proto::Tag {
        sample_index: advance(first, tag.pos() as u64)?,
        key: tag.key().to_owned(),
        value: Some(proto::TagValue { kind: Some(kind) }),
        source_id: None,
    })
}
fn value_from_wire(value: proto::TagValue) -> Result<TagValue> {
    use proto::tag_value::Kind;
    Ok(match value.kind {
        Some(Kind::Float32Value(v)) => TagValue::Float(v),
        Some(Kind::BoolValue(v)) => TagValue::Bool(v),
        Some(Kind::Int64Value(v)) => TagValue::I64(v),
        Some(Kind::Uint64Value(v)) => TagValue::U64(v),
        Some(Kind::StringValue(v)) => TagValue::String(v),
        _ => return Err(err("unsupported IQ tag value")),
    })
}
fn value_kind(value: &proto::TagValue) -> Result<i32> {
    use proto::tag_value::Kind;
    // Map the value's oneof variant to the capability enum used in Open/Started.
    // Inspect by reference so checking a string tag does not clone its contents.
    Ok(match value.kind.as_ref() {
        Some(Kind::Float32Value(_)) => proto::TagKind::Float32 as i32,
        Some(Kind::BoolValue(_)) => proto::TagKind::Bool as i32,
        Some(Kind::Int64Value(_)) => proto::TagKind::Int64 as i32,
        Some(Kind::Uint64Value(_)) => proto::TagKind::Uint64 as i32,
        Some(Kind::StringValue(_)) => proto::TagKind::String as i32,
        _ => return Err(err("unsupported IQ tag value")),
    })
}
