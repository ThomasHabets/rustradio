//! Generate a morse code signal.
use std::borrow::Cow;

use crate::stream::{NCReadStream, NCWriteStream, Tag, TagValue};

/// Generate looping morse code signal.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync_nocopy_tag)]
pub struct MorseEncode {
    #[rustradio(in)]
    src: NCReadStream<String>,

    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
}

const MORSE_AZ_TABLE: [&str; 26] = [
    ".-", "-...", "-.-.", "-..", ".", "..-.", "--.", "....", "..", ".---", "-.-", ".-..", "--",
    "-.", "---", ".--.", "--.-", ".-.", "...", "-", "..-", "...-", ".--", "-..-", "-.--", "--..",
];
const MORSE_DIGIT_TABLE: [&str; 10] = [
    "-----", ".----", "..---", "...--", "....-", ".....", "-....", "--...", "---..", "----.",
];

// TODO: make this spacing configurable.
const DIT: &[u8] = &[1, 0];
const DAH: &[u8] = &[1, 1, 1, 0];
const CHAR_GAP: &[u8] = &[0, 0];
const WORD_GAP: &[u8] = &[0, 0, 0, 0, 0, 0];

fn encode(msg: &str) -> Vec<u8> {
    // Longest normal character may be 5 followed by word boundary => 20+7?
    let mut out = Vec::with_capacity(msg.len() * 32);

    let mut chars = msg.chars().peekable();
    while let Some(c) = chars.next() {
        match c.to_ascii_lowercase() {
            '0'..='9' => {
                let morse = MORSE_DIGIT_TABLE[(c as u8 - b'0') as usize];
                for sym in morse.chars() {
                    out.extend(match sym {
                        '.' => DIT,
                        '-' => DAH,
                        other => panic!("can't happen, got {other}"),
                    });
                }
                // Inter-character gap: 3 zeros, unless next is space or end
                if let Some(next) = chars.peek()
                    && *next != ' '
                {
                    out.extend(CHAR_GAP);
                }
            }
            'a'..='z' => {
                let c = c.to_ascii_lowercase();
                let morse = MORSE_AZ_TABLE[(c as u8 - b'a') as usize];
                for sym in morse.chars() {
                    out.extend(match sym {
                        '.' => DIT,
                        '-' => DAH,
                        other => panic!("can't happen, got {other}"),
                    });
                }
                // Inter-character gap: 3 zeros, unless next is space or end
                if let Some(next) = chars.peek()
                    && *next != ' '
                {
                    out.extend(CHAR_GAP);
                }
            }
            // Inter-word gap: 7 zeros (we already have one 0 from last symbol)
            // TODO: but what about two spaces in a row?
            ' ' => out.extend(WORD_GAP),
            // Probably want a better solution to this.
            other => log::warn!("morse code got invalid character '{other}'. Ignoring"),
        }
    }
    out.extend(WORD_GAP);
    out
}

impl MorseEncode {
    fn process_sync_tags<'a>(&mut self, msg: String, tags: &'a [Tag]) -> (Vec<u8>, Cow<'a, [Tag]>) {
        let mut tags = tags.to_vec();
        tags.push(Tag::new(
            0,
            "MorseEncode::message",
            TagValue::String(msg.clone()),
        ));
        (encode(&msg), Cow::Owned(tags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockRet};
    use crate::stream::new_nocopy_stream;

    #[test]
    fn simple() {
        let (tx, rx) = new_nocopy_stream();
        let (mut b, out) = MorseEncode::new(rx);
        assert!(matches![b.work(), Ok(BlockRet::WaitForStream(_, _))]);
        assert!(out.is_empty());
        for (i, want) in &[
            ("", vec![0, 0, 0, 0, 0, 0]),
            ("A", vec![1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0]),
            (
                "7",
                vec![1, 1, 1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0],
            ),
            (
                "M0THC 73",
                vec![
                    1, 1, 1, 0, 1, 1, 1, 0, 0, 0, // m
                    1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, // 0
                    1, 1, 1, 0, 0, 0, // t
                    1, 0, 1, 0, 1, 0, 1, 0, 0, 0, // h
                    1, 1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, // c EOW
                    1, 1, 1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 0, // 7
                    1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, // 3 EOW
                ],
            ),
            (
                "hello",
                vec![
                    1u8, 0, 1, 0, 1, 0, 1, 0, 0, 0, // h
                    1, 0, 0, 0, // e
                    1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 0, 0, // l
                    1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 0, 0, // l
                    1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, // o EOW.
                ],
            ),
        ] {
            tx.push(i.to_string(), &[]);
            assert!(matches![b.work(), Ok(BlockRet::WaitForStream(_, _))]);
            let (o, _) = out.pop().unwrap();
            assert_eq!(&o, want, "For input {i}");
        }
    }
}
