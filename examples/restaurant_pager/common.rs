//! Protocol constants and encoding shared by the restaurant-pager examples.

pub const SHORT_US: u32 = 204;
pub const LONG_US: u32 = 636;
#[allow(dead_code)]
pub const ROW_GAP_US: u32 = 880;
#[allow(dead_code)]
pub const RESET_US: u32 = 7_312;
pub const FRAME_BITS: usize = 25;

#[cfg(feature = "soapysdr")]
const TX_FRAME_GAP_US: u32 = 1_000;
#[cfg(feature = "soapysdr")]
const TX_RESET_GAP_US: u32 = 8_000;

#[cfg(feature = "soapysdr")]
use std::str::FromStr;

/// Parse a positive microsecond duration for transmit timing options.
#[cfg(feature = "soapysdr")]
fn parse_positive_micros(value: &str) -> std::result::Result<u32, String> {
    let micros = value
        .parse::<u32>()
        .map_err(|error| format!("invalid duration {value:?}: {error}"))?;
    if micros == 0 {
        return Err("duration must be greater than zero".to_string());
    }
    Ok(micros)
}

/// How the transmitter handles the pulse immediately before a frame gap.
#[cfg(feature = "soapysdr")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum PagerTxGapPulse {
    /// The final encoded bit is the last pulse in the frame.
    #[default]
    Data,

    /// Add a short delimiter pulse after the encoded bits.
    Delimiter,
}

#[cfg(feature = "soapysdr")]
impl From<PagerTxGapPulse> for rustradio::blocks::PwmGapPulse {
    fn from(value: PagerTxGapPulse) -> Self {
        match value {
            PagerTxGapPulse::Data => Self::Data,
            PagerTxGapPulse::Delimiter => Self::Delimiter,
        }
    }
}

/// Startup-only PWM timing and framing options shared by both transmitters.
#[cfg(feature = "soapysdr")]
#[derive(Clone, Debug, Eq, PartialEq, clap::Args)]
pub struct PagerTxTiming {
    /// Width of a short transmitted high pulse in microseconds.
    #[arg(long, value_parser = parse_positive_micros, default_value_t = SHORT_US)]
    pub tx_short_us: u32,

    /// Width of a long transmitted high pulse in microseconds.
    #[arg(long, value_parser = parse_positive_micros, default_value_t = LONG_US)]
    pub tx_long_us: u32,

    /// Low gap between repeated frames in microseconds.
    #[arg(long, value_parser = parse_positive_micros, default_value_t = TX_FRAME_GAP_US)]
    pub tx_frame_gap_us: u32,

    /// Final low gap ending a transmission in microseconds.
    #[arg(long, value_parser = parse_positive_micros, default_value_t = TX_RESET_GAP_US)]
    pub tx_reset_gap_us: u32,

    /// Whether the pulse immediately before the frame gap is data or a delimiter.
    #[arg(long, value_enum, default_value = "data")]
    pub tx_gap_pulse: PagerTxGapPulse,
}

#[cfg(feature = "soapysdr")]
impl Default for PagerTxTiming {
    fn default() -> Self {
        Self {
            tx_short_us: SHORT_US,
            tx_long_us: LONG_US,
            tx_frame_gap_us: TX_FRAME_GAP_US,
            tx_reset_gap_us: TX_RESET_GAP_US,
            tx_gap_pulse: PagerTxGapPulse::Data,
        }
    }
}

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

    /// Parse `PAGER [FUNCTION]`, defaulting to the buzz function.
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let mut fields = value.split_whitespace();
        let pager = fields.next().ok_or_else(|| {
            "message must be PAGER or PAGER FUNCTION, such as 11 or 11 sync".to_string()
        })?;
        let function = fields.next().unwrap_or("buzz");
        if fields.next().is_some() {
            return Err(
                "message must be PAGER or PAGER FUNCTION, such as 11 or 11 sync".to_string(),
            );
        }
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
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TimingOpt {
        #[command(flatten)]
        timing: PagerTxTiming,
    }

    /// Verify transmit defaults are compatible with rtl_433's gap thresholds.
    #[test]
    fn parses_transmit_timing_defaults_and_overrides() {
        let defaults = TimingOpt::try_parse_from(["test"]).expect("default timing");
        assert_eq!(defaults.timing, PagerTxTiming::default());
        assert_eq!(defaults.timing.tx_short_us, 204);
        assert_eq!(defaults.timing.tx_long_us, 636);
        assert_eq!(defaults.timing.tx_frame_gap_us, 1_000);
        assert_eq!(defaults.timing.tx_reset_gap_us, 8_000);
        assert_eq!(defaults.timing.tx_gap_pulse, PagerTxGapPulse::Data);

        let custom = TimingOpt::try_parse_from([
            "test",
            "--tx-short-us",
            "250",
            "--tx-long-us",
            "750",
            "--tx-frame-gap-us",
            "6800",
            "--tx-reset-gap-us",
            "10000",
            "--tx-gap-pulse",
            "delimiter",
        ])
        .expect("custom timing");
        assert_eq!(custom.timing.tx_short_us, 250);
        assert_eq!(custom.timing.tx_long_us, 750);
        assert_eq!(custom.timing.tx_frame_gap_us, 6_800);
        assert_eq!(custom.timing.tx_reset_gap_us, 10_000);
        assert_eq!(custom.timing.tx_gap_pulse, PagerTxGapPulse::Delimiter);

        assert!(TimingOpt::try_parse_from(["test", "--tx-short-us", "0"]).is_err());
    }

    /// Verify the default, named, decimal, and hexadecimal message forms.
    #[test]
    fn parses_messages() {
        assert_eq!(
            "11".parse(),
            Ok(PagerMessage {
                pager: 11,
                function: 0x0d,
            })
        );
        assert_eq!(
            "1 sync".parse(),
            Ok(PagerMessage {
                pager: 1,
                function: 0x0f,
            })
        );
        assert_eq!(
            "5 buzz".parse(),
            Ok(PagerMessage {
                pager: 5,
                function: 0x0d,
            })
        );
        assert_eq!(
            "0xf 0x2".parse(),
            Ok(PagerMessage {
                pager: 15,
                function: 2,
            })
        );
        assert!("16 sync".parse::<PagerMessage>().is_err());
        assert!("1 16".parse::<PagerMessage>().is_err());
        assert!("1 sync extra".parse::<PagerMessage>().is_err());
        assert!("11:buzz".parse::<PagerMessage>().is_err());
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
