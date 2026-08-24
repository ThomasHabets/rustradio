//! Protocol constants and encoding shared by the restaurant-pager examples.

pub const SHORT_US: u32 = 204;
pub const LONG_US: u32 = 636;
pub const ROW_GAP_US: u32 = 880;
pub const RESET_US: u32 = 7_312;
pub const FRAME_BITS: usize = 25;

#[cfg(feature = "soapysdr")]
use std::str::FromStr;

/// One pager number and function requested for transmission.
#[cfg(feature = "soapysdr")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PagerMessage {
    pub pager: u8,
    pub function: u8,
}

#[cfg(feature = "soapysdr")]
impl PagerMessage {
    /// Return a readable function name for logging.
    pub fn function_name(&self) -> &'static str {
        match self.function {
            0x0d => "Buzz",
            0x0f => "Sync",
            _ => "Custom",
        }
    }
}

#[cfg(feature = "soapysdr")]
impl FromStr for PagerMessage {
    type Err = String;

    /// Parse `PAGER:FUNCTION`, accepting named or numeric functions.
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (pager, function) = value
            .split_once(':')
            .ok_or_else(|| "message must be PAGER:FUNCTION, such as 11:buzz".to_string())?;
        let pager = parse_integer(pager)?;
        if pager > 0x0f {
            return Err("pager number must be between 0 and 15".to_string());
        }
        let function = match function.to_ascii_lowercase().as_str() {
            "buzz" => 0x0d,
            "sync" => 0x0f,
            _ => parse_integer(function)?,
        };
        if function > 0x0f {
            return Err("pager function must be between 0 and 15".to_string());
        }
        Ok(Self {
            pager: pager as u8,
            function: function as u8,
        })
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal integer.
#[cfg(feature = "soapysdr")]
fn parse_integer(value: &str) -> std::result::Result<u32, String> {
    let hexadecimal = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    match hexadecimal {
        Some(value) => u32::from_str_radix(value, 16),
        None => value.parse(),
    }
    .map_err(|error| format!("invalid integer {value:?}: {error}"))
}

/// Parse and range-check the 16-bit pager-system identifier.
#[cfg(feature = "soapysdr")]
pub fn parse_system_id(value: &str) -> std::result::Result<u16, String> {
    let value = parse_integer(value)?;
    u16::try_from(value).map_err(|_| "system ID must fit in 16 bits".to_string())
}

/// Pack the restaurant-pager fields and return 25 MSB-first bits.
#[cfg(feature = "soapysdr")]
pub fn encode_message(system_id: u16, message: &PagerMessage) -> (u32, Vec<u8>) {
    let raw = (u32::from(system_id) << 9)
        | (u32::from(message.pager) << 5)
        | (u32::from(message.function) << 1)
        | 1;
    let bits = (0..FRAME_BITS)
        .rev()
        .map(|shift| ((raw >> shift) & 1) as u8)
        .collect();
    (raw, bits)
}

#[cfg(all(test, feature = "soapysdr"))]
mod tests {
    use super::*;

    /// Verify named, decimal, and hexadecimal message forms.
    #[test]
    fn parses_messages() {
        assert_eq!(
            "11:buzz".parse(),
            Ok(PagerMessage {
                pager: 11,
                function: 0x0d,
            })
        );
        assert_eq!(
            "0xf:0x2".parse(),
            Ok(PagerMessage {
                pager: 15,
                function: 2,
            })
        );
        assert!("16:sync".parse::<PagerMessage>().is_err());
        assert!("1:16".parse::<PagerMessage>().is_err());
        assert!("buzz".parse::<PagerMessage>().is_err());
    }

    /// Verify the encoded fields match the receiver's bit layout.
    #[test]
    fn encodes_message_layout() {
        let message = PagerMessage {
            pager: 11,
            function: 0x0d,
        };
        let (raw, bits) = encode_message(0xf9bf, &message);
        assert_eq!(bits.len(), FRAME_BITS);
        assert_eq!(bits.last(), Some(&1));
        assert_eq!((raw >> 9) & 0xffff, 0xf9bf);
        assert_eq!((raw >> 5) & 0x0f, 11);
        assert_eq!((raw >> 1) & 0x0f, 0x0d);
    }

    /// Verify system IDs are decimal or hexadecimal 16-bit values.
    #[test]
    fn parses_system_ids() {
        assert_eq!(parse_system_id("65535"), Ok(0xffff));
        assert_eq!(parse_system_id("0xf9bf"), Ok(0xf9bf));
        assert!(parse_system_id("65536").is_err());
        assert!(parse_system_id("not-an-id").is_err());
    }
}
