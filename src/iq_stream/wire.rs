//! Validate protobuf budgets before prost allocates message trees.
use prost::Message;

use super::wire_schema::*;
use super::{MAX_CONTROL, MAX_ENVELOPE, err};
use crate::Result;

/// A decoded envelope with the original encoded Frame size.
#[derive(Debug)]
pub struct Decoded<T> {
    /// Envelope contents.
    pub message: T,
    /// Frame size measured before decoding, excluding its envelope.
    /// Re-encoding cannot recover this size: protobuf decoders discard unknown
    /// fields and normalize encodings, which could hide an oversized wire frame.
    pub frame_bytes: Option<usize>,
}

fn varint(bytes: &[u8], pos: &mut usize) -> Result<u64> {
    let mut v = 0;
    for shift in (0..70).step_by(7) {
        let b = *bytes
            .get(*pos)
            .ok_or_else(|| err("truncated protobuf varint"))?;
        *pos += 1;
        if shift == 63 && b > 1 {
            return Err(err("protobuf varint overflow"));
        }
        v |= u64::from(b & 127) << shift;
        if b < 128 {
            return Ok(v);
        }
    }
    Err(err("protobuf varint overflow"))
}
struct Budget {
    nodes: usize,
    frame_bytes: Option<usize>,
}
fn inspect(
    bytes: &[u8],
    schema: usize,
    depth: usize,
    tags: usize,
    budget: &mut Budget,
) -> Result<()> {
    // Message wrappers add protobuf depth without adding TagValue depth. Bound
    // both separately so legitimate nested dictionaries fit while malformed or
    // excessively deep messages are rejected before prost allocates their tree.
    if depth > 256 {
        return Err(err("protobuf recursion limit"));
    }
    let tags = tags + usize::from(schema == TAG_VALUE);
    if tags > 32 {
        return Err(err("IQ tag nesting limit"));
    }
    if schema == TAG_VALUE {
        budget.nodes += 1;
        if budget.nodes > 16384 {
            return Err(err("IQ tag node limit"));
        }
    }
    if schema == FRAME {
        if bytes.len() > 1024 * 1024 {
            return Err(err("IQ frame byte limit"));
        }
        budget.frame_bytes = Some(bytes.len());
    }
    let fields = SCHEMA[schema];
    // Bit positions refer to this message's schema entries and oneof groups.
    // Reject duplicate singular fields instead of accepting protobuf's usual
    // merge/last-value behavior, which could make budgets or bodies ambiguous.
    let mut seen = 0u64;
    let mut oneofs = 0u64;
    let mut pos = 0;
    let mut tag_count = 0;
    while pos < bytes.len() {
        let key = varint(bytes, &mut pos)?;
        let number = u32::try_from(key >> 3).map_err(|_| err("invalid protobuf field"))?;
        if number == 0 || number > 0x1fff_ffff {
            return Err(err("invalid protobuf field number"));
        }
        let field = fields.iter().position(|f| f.0 == number);
        if let Some(index) = field {
            let (_, _, repeated, group) = fields[index];
            if !repeated && seen & (1 << index) != 0 {
                return Err(err("duplicate protobuf scalar/message field"));
            }
            seen |= 1 << index;
            if group > 0 {
                if oneofs & (1 << group) != 0 {
                    return Err(err("multiple protobuf oneof values"));
                }
                oneofs |= 1 << group;
            }
        }
        let length = match key & 7 {
            0 => {
                varint(bytes, &mut pos)?;
                0
            }
            1 => 8,
            2 => usize::try_from(varint(bytes, &mut pos)?)
                .map_err(|_| err("protobuf length overflow"))?,
            5 => 4,
            _ => return Err(err("unsupported protobuf wire type")),
        };
        let end = pos
            .checked_add(length)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| err("truncated protobuf field"))?;
        if let Some(index) = field {
            let nested = fields[index].1;
            if nested == TAG {
                tag_count += 1;
                if tag_count > 4096 {
                    return Err(err("IQ tag count limit"));
                }
            }
            if nested != 0 {
                if key & 7 != 2 {
                    return Err(err("incorrect protobuf message wire type"));
                }
                inspect(&bytes[pos..end], nested, depth + 1, tags, budget)?;
            }
        }
        pos = end;
    }
    Ok(())
}
pub(super) fn decode<T: Message + Default>(bytes: &[u8], client: bool) -> Result<Decoded<T>> {
    if bytes.len() > MAX_ENVELOPE {
        return Err(err("IQ envelope byte limit"));
    }
    let mut budget = Budget {
        nodes: 0,
        frame_bytes: None,
    };
    inspect(
        bytes,
        if client {
            CLIENT_MESSAGE
        } else {
            SERVER_MESSAGE
        },
        0,
        0,
        &mut budget,
    )?;
    if budget.frame_bytes.is_none() && bytes.len() > MAX_CONTROL {
        return Err(err("IQ control byte limit"));
    }
    // Prost's default recursion limit is disabled for this feature because tag
    // containers add wrapper messages. The scan above enforces our own limits
    // before this decode can allocate strings, payloads, or nested values.
    let message = T::decode(bytes).map_err(|e| err(format!("invalid IQ protobuf: {e}")))?;
    Ok(Decoded {
        message,
        frame_bytes: budget.frame_bytes,
    })
}
/// Decode a binary WebSocket client envelope, enforcing allocation budgets.
pub fn decode_client(bytes: &[u8]) -> Result<Decoded<super::proto::ClientMessage>> {
    decode(bytes, true)
}
/// Decode a binary WebSocket server envelope, enforcing allocation budgets.
pub fn decode_server(bytes: &[u8]) -> Result<Decoded<super::proto::ServerMessage>> {
    decode(bytes, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_envelope_bodies_and_truncation() {
        assert!(decode_client(&[0x0a, 0, 0x0a, 0]).is_err());
        assert!(decode_client(&[0x0a, 8]).is_err());
        assert!(decode_client(&[0]).is_err());
    }
    #[test]
    fn rejects_oversized_controls_before_decode() {
        assert!(decode_client(&vec![0; MAX_CONTROL + 1]).is_err());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::iq_stream::proto;
    use prost::Message;
    fn envelope(value: proto::TagValue) -> Vec<u8> {
        proto::ServerMessage {
            body: Some(proto::server_message::Body::Started(proto::Started {
                description: Some(proto::StreamDescription {
                    properties: vec![proto::Property {
                        key: "test".into(),
                        value: Some(value),
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
        .encode_to_vec()
    }
    fn dictionary(depth: usize) -> proto::TagValue {
        let mut value = proto::TagValue {
            kind: Some(proto::tag_value::Kind::BoolValue(false)),
        };
        for _ in 1..depth {
            value = proto::TagValue {
                kind: Some(proto::tag_value::Kind::DictionaryValue(
                    proto::DictionaryValue {
                        entries: vec![proto::DictionaryEntry {
                            key: Some(proto::TagValue {
                                kind: Some(proto::tag_value::Kind::StringValue("key".into())),
                            }),
                            value: Some(value),
                        }],
                    },
                )),
            };
        }
        value
    }
    #[test]
    fn tag_depth_is_checked_before_message_allocation() {
        assert!(decode_server(&envelope(dictionary(32))).is_ok());
        assert!(decode_server(&envelope(dictionary(33))).is_err());
    }
    #[test]
    fn tag_count_includes_entries_without_values() {
        let bytes = proto::ServerMessage {
            body: Some(proto::server_message::Body::Frame(proto::Frame {
                sequence: 0,
                body: Some(proto::frame::Body::Chunk(proto::SampleChunk {
                    sample_count: 1,
                    samples: vec![0; 4],
                    tags: vec![proto::Tag::default(); 4097],
                    ..Default::default()
                })),
            })),
        }
        .encode_to_vec();
        assert!(decode_server(&bytes).is_err());
    }
    #[test]
    fn total_nodes_are_bounded_even_for_shallow_values() {
        let value = proto::TagValue {
            kind: Some(proto::tag_value::Kind::ListValue(proto::SequenceValue {
                values: vec![
                    proto::TagValue {
                        kind: Some(proto::tag_value::Kind::BoolValue(true))
                    };
                    16384
                ],
            })),
        };
        assert!(
            decode_server(&envelope(value))
                .unwrap_err()
                .to_string()
                .contains("node limit")
        );
    }
}
