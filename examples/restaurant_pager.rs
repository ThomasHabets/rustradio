//! Decode restaurant guest pagers from a complex-float I/Q capture.
//!
//! The supported protocol is the 25-bit EV1527 variant described by rtl_433's
//! [`restaurant_pager.conf`][protocol]: short OOK pulses are one bits, long
//! pulses are zero bits, and every frame ends with a delimiter pulse followed
//! by a long gap.
//!
//! ```text
//! cargo run --release --example restaurant_pager -- \
//!     file data.c32
//! cargo run -F soapysdr --example restaurant_pager -- \
//!     --threshold 0.1 \
//!     soapy-sdr driver=lime --freq 2.45G --igain 0.7
//! cargo run -F soapysdr --example restaurant_pager -- \
//!     soapy-sdr driver=lime --freq 433.92M --interactive \
//!     --system-id 0xf9bf --tx-gain 0.1
//! ```
//!
//! At the interactive prompt, enter a pager number to buzz it, or add a
//! function such as `1 sync` or `5 buzz`. Use `system-id 0xabcd` to change the
//! system ID. Enter `help` for syntax and `quit` to stop. Ensure the selected
//! frequency and any transmission are legal in your location.
//!
//! [protocol]: https://github.com/jflaflamme/rtl_433/blob/1b5550e75a2c1f483db1fb29e80173356bbb74be/conf/restaurant_pager.conf

use std::path::PathBuf;

use anyhow::{Result, ensure};
use clap::Parser;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::{ComplexToMag2, FileSource, PwmDecoder, PwmFrame, PwmGapPulse};
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::stream::{NCReadStream, ReadStream};
use rustradio::{Complex, Float};

#[cfg(feature = "soapysdr")]
use rustradio::blocks::{Map, PwmEncoder, SoapySdrSink};
#[cfg(feature = "soapysdr")]
use rustradio::graph::CancellationToken;
#[cfg(feature = "soapysdr")]
use rustradio::stream::{NCWriteStream, Tag, TagValue, new_nocopy_stream};
#[cfg(feature = "soapysdr")]
use rustyline::completion::{Completer, Pair};
#[cfg(feature = "soapysdr")]
use rustyline::error::ReadlineError;
#[cfg(feature = "soapysdr")]
use rustyline::highlight::Highlighter;
#[cfg(feature = "soapysdr")]
use rustyline::hint::Hinter;
#[cfg(feature = "soapysdr")]
use rustyline::history::DefaultHistory;
#[cfg(feature = "soapysdr")]
use rustyline::validate::Validator;
#[cfg(feature = "soapysdr")]
use rustyline::{Cmd, Context, Editor, Event, Helper, KeyEvent};

#[path = "restaurant_pager/common.rs"]
mod common;
use common::{FRAME_BITS, LONG_US, RESET_US, ROW_GAP_US, SHORT_US};
#[cfg(feature = "soapysdr")]
use common::{PagerMessage, encode_message, parse_system_id};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Capture sample rate in samples per second.
    #[arg(long, default_value_t = 125_000)]
    sample_rate: u32,

    /// Verbosity level.
    #[arg(short, value_parser=rustradio::parse_verbosity, default_value = "info")]
    verbose: usize,

    /// OOK power threshold (the input is magnitude squared).
    #[arg(long, default_value_t = 0.01)]
    threshold: Float,

    /// Number of identical frames required in one transmission.
    #[arg(long, default_value_t = 3)]
    repeats: usize,

    #[command(subcommand)]
    source: Source,
}

#[derive(clap::Args, Debug)]
struct FileOpt {
    /// Raw little-endian complex-f32 I/Q capture.
    input: PathBuf,
}

#[cfg(feature = "soapysdr")]
#[derive(clap::Args, Debug)]
struct SoapyOpt {
    /// RF center frequency in Hz.
    #[arg(long, value_parser=rustradio::parse_frequency)]
    freq: f64,

    /// Normalized receive gain from zero through one.
    #[arg(long, default_value_t = 0.3)]
    igain: f64,

    /// SoapySDR driver string.
    driver: String,

    /// Start a command prompt for transmitting pager messages.
    #[arg(long)]
    interactive: bool,

    /// Pager system identifier, in decimal or hexadecimal.
    #[arg(long, value_parser = parse_system_id, default_value = "0xf9bf")]
    system_id: u16,

    /// Normalized transmit gain from zero through one.
    #[arg(long, default_value_t = 0.1)]
    tx_gain: f64,

    /// Complex baseband amplitude while the OOK carrier is on.
    #[arg(long, default_value_t = 0.5)]
    tx_amplitude: Float,

    /// Number of identical frames sent for each message.
    #[arg(long, default_value_t = 8)]
    tx_repeats: usize,

    /// SoapySDR transmit channel.
    #[arg(long, default_value_t = 0)]
    tx_channel: usize,

    /// SoapySDR transmit antenna name.
    #[arg(long)]
    tx_antenna: Option<String>,
}

#[derive(clap::Subcommand, Debug)]
enum Source {
    File(FileOpt),
    #[cfg(feature = "soapysdr")]
    SoapySdr(SoapyOpt),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DecodedTransmission {
    raw: u32,
    repeats: usize,
    first_sample: u64,
    sample_rate: u32,
}

impl DecodedTransmission {
    /// Interpret a generic PWM frame as a restaurant-pager message.
    fn from_frame(frame: PwmFrame, sample_rate: u32) -> Option<Self> {
        if frame.len() != FRAME_BITS || frame.bits().last() != Some(&1) {
            return None;
        }
        let raw = frame
            .bits()
            .iter()
            .fold(0_u32, |value, &bit| (value << 1) | u32::from(bit));
        Some(Self {
            raw,
            repeats: frame.repeats(),
            first_sample: frame.first_sample(),
            sample_rate,
        })
    }

    /// Return the pager system identifier.
    fn system_id(&self) -> u16 {
        ((self.raw >> 9) & 0xffff) as u16
    }

    /// Return the addressed pager number.
    fn pager(&self) -> u8 {
        ((self.raw >> 5) & 0x0f) as u8
    }

    /// Return the requested pager function code.
    fn function(&self) -> u8 {
        ((self.raw >> 1) & 0x0f) as u8
    }

    /// Return a readable name for the pager function.
    fn function_name(&self) -> &'static str {
        match self.function() {
            0x0d => "Buzz",
            0x0f => "Sync",
            _ => "Unknown",
        }
    }
}

impl std::fmt::Display for DecodedTransmission {
    /// Format the decoded pager fields and capture timestamp.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.first_sample as f64 / f64::from(self.sample_rate);
        write!(
            f,
            "Restaurant-Pager: id=0x{:04x} pager={} function={} (0x{:x}) \
             repeats={} raw=0x{:07x} time={seconds:.6}s",
            self.system_id(),
            self.pager(),
            self.function_name(),
            self.function(),
            self.repeats,
            self.raw,
        )
    }
}

/// Convert a duration in microseconds to a rounded input-sample count.
fn us_to_samples(sample_rate: u32, micros: u32) -> usize {
    ((u64::from(sample_rate) * u64::from(micros) + 500_000) / 1_000_000)
        .max(1)
        .try_into()
        .expect("sample count does not fit in usize")
}

/// Print decoded messages without coupling console I/O to the DSP block.
enum PagerOutput {
    Stdout,
    #[cfg(feature = "soapysdr")]
    Rustyline(Box<dyn rustyline::ExternalPrinter + Send>),
}

impl PagerOutput {
    /// Print a decoded message, preserving an active Rustyline prompt.
    fn print(&mut self, message: String) {
        match self {
            Self::Stdout => println!("{message}"),
            #[cfg(feature = "soapysdr")]
            Self::Rustyline(printer) => {
                if let Err(error) = printer.print(message.clone()) {
                    eprintln!("interactive output error: {error}");
                    println!("{message}");
                }
            }
        }
    }
}

/// Print decoded messages without coupling console I/O to the DSP block.
#[derive(rustradio_macros::Block)]
#[rustradio(new)]
struct RestaurantPagerPrinter {
    #[rustradio(in)]
    src: NCReadStream<PwmFrame>,
    sample_rate: u32,
    output: PagerOutput,
}

impl Block for RestaurantPagerPrinter {
    /// Print all decoded frames currently available on the input stream.
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        loop {
            let Some((frame, _tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            if let Some(decoded) = DecodedTransmission::from_frame(frame, self.sample_rate) {
                self.output.print(decoded.to_string());
            }
        }
    }
}

struct SourceOutput {
    samples: ReadStream<Complex>,
    #[cfg(feature = "soapysdr")]
    device: Option<soapysdr::Device>,
}

fn source(opt: &Opt, g: &mut impl GraphRunner) -> Result<SourceOutput> {
    Ok(match opt.source {
        Source::File(ref o) => {
            let (b, prev) = FileSource::<Complex>::new(&o.input)?;
            g.add(Box::new(b));
            SourceOutput {
                samples: prev,
                #[cfg(feature = "soapysdr")]
                device: None,
            }
        }
        #[cfg(feature = "soapysdr")]
        Source::SoapySdr(ref o) => {
            let dev = soapysdr::Device::new(&*o.driver)?;
            let (b, prev) =
                rustradio::blocks::SoapySdrSource::builder(&dev, o.freq, opt.sample_rate.into())
                    .igain(o.igain)?
                    .build()?;
            g.add(Box::new(b));
            SourceOutput {
                samples: prev,
                device: Some(dev),
            }
        }
    })
}

#[cfg(feature = "soapysdr")]
#[derive(Debug, Eq, PartialEq)]
enum PromptCommand {
    Empty,
    Help,
    Quit,
    SetSystemId(u16),
    Send(PagerMessage),
}

#[cfg(feature = "soapysdr")]
const PROMPT_HELP: &str = "\
Commands:
  PAGER                  Buzz the specified pager (0-15)
  PAGER FUNCTION         Send buzz, sync, or a numeric function (0-15)
  system-id HEX          Change the 16-bit system ID
  help                   Show this help
  quit | exit            Stop the transmitter (Ctrl-D also works)
  Ctrl-X Ctrl-R          Redraw the current input line

Examples:
  1
  1 sync
  5 buzz
  system-id 0xabcd";

#[cfg(feature = "soapysdr")]
const PROMPT_COMMAND_COMPLETIONS: &[(&str, &str)] = &[
    ("help", "help"),
    ("quit", "quit"),
    ("exit", "exit"),
    ("system-id", "system-id "),
];

#[cfg(feature = "soapysdr")]
const PROMPT_FUNCTION_COMPLETIONS: &[(&str, &str)] = &[("buzz", "buzz"), ("sync", "sync")];

#[cfg(feature = "soapysdr")]
struct PagerPromptHelper;

#[cfg(feature = "soapysdr")]
type PagerEditor = Editor<PagerPromptHelper, DefaultHistory>;

/// Return completion candidates for the word at the cursor.
#[cfg(feature = "soapysdr")]
fn prompt_completions(line: &str, pos: usize) -> (usize, Vec<Pair>) {
    let before_cursor = &line[..pos];
    let word_start = before_cursor
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8());
    let word = &before_cursor[word_start..];
    let preceding_words: Vec<_> = before_cursor[..word_start].split_whitespace().collect();

    let choices = match preceding_words.as_slice() {
        [] => PROMPT_COMMAND_COMPLETIONS,
        [pager] if pager.parse::<PagerMessage>().is_ok() => PROMPT_FUNCTION_COMPLETIONS,
        _ => &[],
    };
    let candidates = choices
        .iter()
        .filter(|(display, _)| {
            display
                .get(..word.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(word))
        })
        .map(|(display, replacement)| Pair {
            display: (*display).to_string(),
            replacement: (*replacement).to_string(),
        })
        .collect();
    (word_start, candidates)
}

#[cfg(feature = "soapysdr")]
impl Completer for PagerPromptHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        Ok(prompt_completions(line, pos))
    }
}

#[cfg(feature = "soapysdr")]
impl Hinter for PagerPromptHelper {
    type Hint = String;
}

#[cfg(feature = "soapysdr")]
impl Highlighter for PagerPromptHelper {}

#[cfg(feature = "soapysdr")]
impl Validator for PagerPromptHelper {}

#[cfg(feature = "soapysdr")]
impl Helper for PagerPromptHelper {}

/// Parse a 16-bit hexadecimal pager-system identifier.
#[cfg(feature = "soapysdr")]
fn parse_hex_system_id(value: &str) -> std::result::Result<u16, String> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if digits.is_empty() {
        return Err("system ID must contain hexadecimal digits".to_string());
    }
    u16::from_str_radix(digits, 16)
        .map_err(|error| format!("invalid hexadecimal system ID {value:?}: {error}"))
}

/// Parse one interactive command without terminating the prompt on errors.
#[cfg(feature = "soapysdr")]
fn parse_prompt_command(line: &str) -> std::result::Result<PromptCommand, String> {
    let line = line.trim();
    match line.to_ascii_lowercase().as_str() {
        "" => Ok(PromptCommand::Empty),
        "help" => Ok(PromptCommand::Help),
        "quit" | "exit" => Ok(PromptCommand::Quit),
        _ => {
            let mut fields = line.split_whitespace();
            if fields
                .next()
                .is_some_and(|command| command.eq_ignore_ascii_case("system-id"))
            {
                let value = fields
                    .next()
                    .ok_or_else(|| "system-id requires a hexadecimal value".to_string())?;
                if fields.next().is_some() {
                    return Err("system-id accepts exactly one hexadecimal value".to_string());
                }
                parse_hex_system_id(value).map(PromptCommand::SetSystemId)
            } else {
                line.parse().map(PromptCommand::Send)
            }
        }
    }
}

/// Push one encoded message into the transmitter queue.
#[cfg(feature = "soapysdr")]
fn queue_message(packets: &NCWriteStream<Vec<u8>>, system_id: u16, message: PagerMessage) {
    if packets.remaining() == 0 {
        eprintln!("transmit queue is full; message was not queued");
        return;
    }
    let (raw, bits) = encode_message(system_id, &message);
    println!(
        "Queueing id=0x{system_id:04x} pager={} function={} (0x{:x}) raw=0x{raw:07x}",
        message.pager,
        message.function_name(),
        message.function,
    );
    packets.push(
        bits,
        vec![Tag::new(
            0,
            "RestaurantPagerTx::message",
            TagValue::String(format!(
                "id=0x{system_id:04x} pager={} function=0x{:x}",
                message.pager, message.function,
            )),
        )],
    );
}

/// Run the Rustyline editor and feed accepted messages to the graph.
#[cfg(feature = "soapysdr")]
fn prompt_loop(
    mut editor: PagerEditor,
    packets: NCWriteStream<Vec<u8>>,
    cancel: CancellationToken,
    mut system_id: u16,
) {
    println!("Interactive transmitter ready; enter `help` for commands");
    loop {
        match editor.readline("pager> ") {
            Ok(line) => {
                if !line.trim().is_empty()
                    && let Err(error) = editor.add_history_entry(line.as_str())
                {
                    eprintln!("could not add prompt history: {error}");
                }
                match parse_prompt_command(&line) {
                    Ok(PromptCommand::Empty) => {}
                    Ok(PromptCommand::Help) => println!("{PROMPT_HELP}"),
                    Ok(PromptCommand::Quit) => {
                        cancel.cancel();
                        break;
                    }
                    Ok(PromptCommand::SetSystemId(value)) => {
                        system_id = value;
                        println!("System ID set to 0x{system_id:04x}");
                    }
                    Ok(PromptCommand::Send(message)) => {
                        queue_message(&packets, system_id, message);
                    }
                    Err(error) => eprintln!("invalid command: {error}"),
                }
            }
            Err(ReadlineError::Interrupted) => {}
            Err(ReadlineError::Eof) => {
                cancel.cancel();
                break;
            }
            Err(error) => {
                eprintln!("interactive input error: {error}");
                cancel.cancel();
                break;
            }
        }
    }
}

/// Add the interactive transmitter branch and start its input thread.
#[cfg(feature = "soapysdr")]
fn add_interactive_transmitter(
    graph: &mut impl GraphRunner,
    device: &soapysdr::Device,
    opt: &Opt,
    soapy: &SoapyOpt,
) -> Result<(PagerOutput, std::thread::JoinHandle<()>)> {
    ensure!(
        (0.0..=1.0).contains(&soapy.tx_gain) && soapy.tx_gain.is_finite(),
        "transmit gain must be finite and between zero and one",
    );
    ensure!(
        soapy.tx_amplitude > 0.0 && soapy.tx_amplitude <= 1.0 && soapy.tx_amplitude.is_finite(),
        "transmit amplitude must be finite, greater than zero, and no greater than one",
    );
    ensure!(
        soapy.tx_repeats > 0,
        "transmit repeat count must be greater than zero"
    );

    let (packets, packet_stream) = new_nocopy_stream();
    let (encoder, envelope) = PwmEncoder::builder(
        us_to_samples(opt.sample_rate, SHORT_US),
        us_to_samples(opt.sample_rate, LONG_US),
        us_to_samples(opt.sample_rate, ROW_GAP_US),
        us_to_samples(opt.sample_rate, RESET_US),
    )
    .repeats(soapy.tx_repeats)
    .max_frame_bits(FRAME_BITS)
    .gap_pulse(PwmGapPulse::Delimiter)
    .build(packet_stream)?;
    graph.add(Box::new(encoder));

    let amplitude = soapy.tx_amplitude;
    let (to_complex, samples) = Map::keep_tags(envelope, "OokToComplex", move |level| {
        Complex::new(amplitude * level, 0.0)
    });
    graph.add(Box::new(to_complex));

    let mut sink = SoapySdrSink::builder(device, soapy.freq, f64::from(opt.sample_rate))
        .channel(soapy.tx_channel)
        .ogain(soapy.tx_gain)?;
    if let Some(antenna) = &soapy.tx_antenna {
        sink = sink.antenna(antenna.clone());
    }
    graph.add(Box::new(sink.build(samples)?));

    let mut editor = PagerEditor::new()?;
    editor.set_helper(Some(PagerPromptHelper));
    let _ = editor.bind_sequence(
        Event::KeySeq(vec![KeyEvent::ctrl('X'), KeyEvent::ctrl('R')]),
        Cmd::Repaint,
    );
    let printer = editor.create_external_printer()?;
    let output = PagerOutput::Rustyline(Box::new(printer));
    let cancel = graph.cancel_token();
    let system_id = soapy.system_id;
    let prompt = std::thread::Builder::new()
        .name("restaurant-pager-prompt".to_string())
        .spawn(move || prompt_loop(editor, packets, cancel, system_id))?;
    Ok((output, prompt))
}

/// Build and run the restaurant-pager decoding graph.
fn main() -> Result<()> {
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .module("soapysdr")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    //soapysdr::configure_logging();

    ensure!(opt.sample_rate > 0, "sample rate must be greater than zero");
    let mut graph = MTGraph::new();

    let source = source(&opt, &mut graph)?;
    #[cfg(feature = "soapysdr")]
    let (output, prompt) = match &opt.source {
        Source::SoapySdr(soapy) if soapy.interactive => {
            let device = source
                .device
                .as_ref()
                .expect("SoapySDR source must retain its device");
            let (output, prompt) = add_interactive_transmitter(&mut graph, device, &opt, soapy)?;
            (output, Some(prompt))
        }
        _ => (PagerOutput::Stdout, None),
    };
    #[cfg(not(feature = "soapysdr"))]
    let output = PagerOutput::Stdout;

    let prev = source.samples;
    let prev = rustradio::blockchain![
        graph,
        prev,
        ComplexToMag2::new(prev),
        PwmDecoder::builder(
            opt.threshold,
            us_to_samples(opt.sample_rate, SHORT_US),
            us_to_samples(opt.sample_rate, LONG_US),
            us_to_samples(opt.sample_rate, ROW_GAP_US),
            us_to_samples(opt.sample_rate, RESET_US),
        )
        .frame_bits(Some(FRAME_BITS))
        .min_repeats(opt.repeats)
        .gap_pulse(PwmGapPulse::Delimiter)
        .build(prev)?,
    ];
    graph.add(Box::new(RestaurantPagerPrinter::new(
        prev,
        opt.sample_rate,
        output,
    )));

    let run_result = graph.run();
    #[cfg(feature = "soapysdr")]
    if run_result.is_ok()
        && let Some(prompt) = prompt
    {
        prompt
            .join()
            .map_err(|_| anyhow::anyhow!("interactive prompt thread panicked"))?;
    }
    run_result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the decoded bit fields match the restaurant-pager layout.
    #[test]
    fn decoded_fields() {
        let target = (0xf9bf_u32 << 9) | (11 << 5) | (0x0d << 1) | 1;
        let decoded = DecodedTransmission {
            raw: target,
            repeats: 3,
            first_sample: 0,
            sample_rate: 125_000,
        };
        assert_eq!(decoded.system_id(), 0xf9bf);
        assert_eq!(decoded.pager(), 11);
        assert_eq!(decoded.function(), 0x0d);
        assert_eq!(decoded.function_name(), "Buzz");
    }

    /// Verify interactive commands and messages are distinguished.
    #[cfg(feature = "soapysdr")]
    #[test]
    fn parses_prompt_commands() {
        assert_eq!(parse_prompt_command(""), Ok(PromptCommand::Empty));
        assert_eq!(parse_prompt_command(" HELP "), Ok(PromptCommand::Help));
        assert_eq!(parse_prompt_command("exit"), Ok(PromptCommand::Quit));
        assert_eq!(
            parse_prompt_command("1"),
            Ok(PromptCommand::Send(PagerMessage {
                pager: 1,
                function: 0x0d,
            }))
        );
        assert_eq!(
            parse_prompt_command("1 sync"),
            Ok(PromptCommand::Send(PagerMessage {
                pager: 1,
                function: 0x0f,
            }))
        );
        assert_eq!(
            parse_prompt_command("5 buzz"),
            Ok(PromptCommand::Send(PagerMessage {
                pager: 5,
                function: 0x0d,
            }))
        );
        assert_eq!(
            parse_prompt_command("SYSTEM-ID 0xabcd"),
            Ok(PromptCommand::SetSystemId(0xabcd))
        );
        assert_eq!(
            parse_prompt_command("system-id f9bf"),
            Ok(PromptCommand::SetSystemId(0xf9bf))
        );
        assert!(parse_prompt_command("system-id").is_err());
        assert!(parse_prompt_command("system-id 0x1234 extra").is_err());
        assert!(parse_prompt_command("system-id 0x").is_err());
        assert!(parse_prompt_command("system-id 10000").is_err());
        assert!(parse_prompt_command("11:buzz").is_err());
        assert!(parse_prompt_command("not a message").is_err());
    }

    /// Verify tab completion follows the command grammar at the cursor.
    #[cfg(feature = "soapysdr")]
    #[test]
    fn completes_prompt_commands() {
        fn completions(line: &str) -> (usize, Vec<(String, String)>) {
            let (start, candidates) = prompt_completions(line, line.len());
            (
                start,
                candidates
                    .into_iter()
                    .map(|candidate| (candidate.display, candidate.replacement))
                    .collect(),
            )
        }

        assert_eq!(
            completions(""),
            (
                0,
                vec![
                    ("help".to_string(), "help".to_string()),
                    ("quit".to_string(), "quit".to_string()),
                    ("exit".to_string(), "exit".to_string()),
                    ("system-id".to_string(), "system-id ".to_string()),
                ],
            )
        );
        assert_eq!(
            completions("S"),
            (0, vec![("system-id".to_string(), "system-id ".to_string())],)
        );
        assert_eq!(
            completions("1 "),
            (
                2,
                vec![
                    ("buzz".to_string(), "buzz".to_string()),
                    ("sync".to_string(), "sync".to_string()),
                ],
            )
        );
        assert_eq!(
            completions("5 s"),
            (2, vec![("sync".to_string(), "sync".to_string())])
        );
        assert_eq!(completions("system-id "), (10, Vec::new()));
        assert_eq!(completions("16 "), (3, Vec::new()));
    }
}
