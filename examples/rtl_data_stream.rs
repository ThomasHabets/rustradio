//! RTL-SDR source that serves downsampled data over the DATA_STREAM.md protocol.
//!
//! The transport is stdin/stdout. Control packets are read from stdin and data
//! packets are written to stdout.
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use anyhow::{Result, bail};
use clap::Parser;

use rustradio::block::{Block, BlockEOF, BlockName, BlockRet};
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::{CancellationToken, Graph, GraphRunner};
use rustradio::stream::ReadStream;

const PROTOCOL_VERSION: u32 = 0;
const PACKET_VERSION: u8 = 1;
const PACKET_REQUEST_DATA: u8 = 2;
const PACKET_DATA: u8 = 3;
const MAX_CONTROL_PACKET_LEN: usize = 1 << 20;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Tuned frequency, if reading from RTL SDR.
    #[arg(long = "freq", default_value_t = 100_000_000)]
    freq: u64,

    /// Verbosity of debug messages.
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// Input gain, if reading from RTL SDR.
    #[arg(long = "gain", default_value = "20")]
    gain: i32,

    /// Protocol stream identifier for the downsampled RTL-SDR byte stream.
    #[arg(long = "stream-id", default_value = "rtl-sdr")]
    stream_id: String,

    /// Maximum data bytes to put in one protocol Data packet.
    #[arg(long = "packet-bytes", default_value_t = 16_384)]
    packet_bytes: usize,
}

enum Control {
    RequestData { stream_id: String, window: usize },
}

struct DataStreamSink {
    src: ReadStream<u8>,
    writer: Box<dyn Write + Send>,
    control: Receiver<Control>,
    stream_id: String,
    window: usize,
    max_packet_data: usize,
}

impl DataStreamSink {
    fn new<W: Write + Send + 'static>(
        src: ReadStream<u8>,
        writer: W,
        control: Receiver<Control>,
        stream_id: String,
        max_packet_data: usize,
    ) -> Self {
        Self {
            src,
            writer: Box::new(writer),
            control,
            stream_id,
            window: 0,
            max_packet_data,
        }
    }

    fn update_window(&mut self) {
        while let Ok(Control::RequestData { stream_id, window }) = self.control.try_recv() {
            if stream_id == self.stream_id {
                self.window = window;
            }
        }
    }

    fn write_data_packet(&mut self, data: &[u8]) -> rustradio::Result<()> {
        let stream_id = self.stream_id.as_bytes();
        let packet_len = 1usize
            .checked_add(4)
            .and_then(|n| n.checked_add(stream_id.len()))
            .and_then(|n| n.checked_add(data.len()))
            .ok_or_else(|| rustradio::Error::msg("data packet length overflow"))?;
        let packet_len = u32::try_from(packet_len)
            .map_err(|_| rustradio::Error::msg("data packet is too large"))?;
        let stream_id_len = u32::try_from(stream_id.len())
            .map_err(|_| rustradio::Error::msg("stream ID is too large"))?;

        self.writer.write_all(&packet_len.to_le_bytes())?;
        self.writer.write_all(&[PACKET_DATA])?;
        self.writer.write_all(&stream_id_len.to_le_bytes())?;
        self.writer.write_all(stream_id)?;
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }
}

impl BlockName for DataStreamSink {
    fn block_name(&self) -> &str {
        "DataStreamSink"
    }
}

impl BlockEOF for DataStreamSink {
    fn eof(&mut self) -> bool {
        self.src.eof()
    }
}

impl Block for DataStreamSink {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        self.update_window();
        if self.window == 0 {
            return Ok(BlockRet::Pending);
        }

        let (input, _tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 2));
        }

        let n = input.len().min(self.window).min(self.max_packet_data) & !1usize;
        if n == 0 {
            return Ok(BlockRet::Pending);
        }

        self.write_data_packet(&input.slice()[..n])?;
        input.consume(n);
        self.window -= n;
        Ok(BlockRet::Again)
    }
}

fn write_packet<W: Write>(writer: &mut W, packet_type: u8, payload: &[u8]) -> Result<()> {
    let packet_len = u32::try_from(1usize + payload.len())?;
    writer.write_all(&packet_len.to_le_bytes())?;
    writer.write_all(&[packet_type])?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

fn write_version<W: Write>(writer: &mut W) -> Result<()> {
    write_packet(writer, PACKET_VERSION, &PROTOCOL_VERSION.to_le_bytes())
}

fn read_packet<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match reader.read(&mut len[..1]) {
        Ok(0) => return Ok(None),
        Ok(1) => reader.read_exact(&mut len[1..])?,
        Ok(_) => unreachable!("one byte buffer cannot read more than one byte"),
        Err(e) => return Err(e.into()),
    }

    let len = u32::from_le_bytes(len) as usize;
    if len == 0 {
        bail!("protocol packet length must include a packet type byte");
    }
    if len > MAX_CONTROL_PACKET_LEN {
        bail!("protocol packet length {len} exceeds limit {MAX_CONTROL_PACKET_LEN}");
    }

    let mut packet = vec![0u8; len];
    reader.read_exact(&mut packet)?;
    Ok(Some(packet))
}

fn parse_version(packet: &[u8]) -> Result<()> {
    if packet.len() != 5 {
        bail!("version packet has length {}, want 5", packet.len());
    }
    let version = u32::from_le_bytes(packet[1..5].try_into()?);
    if version != PROTOCOL_VERSION {
        bail!("unsupported protocol version {version}");
    }
    Ok(())
}

fn parse_request_data(packet: &[u8]) -> Result<Control> {
    if packet.len() < 5 {
        bail!(
            "RequestData packet has length {}, want at least 5",
            packet.len()
        );
    }
    let window = u32::from_le_bytes(packet[1..5].try_into()?) as usize;
    let stream_id = String::from_utf8(packet[5..].to_vec())?;
    Ok(Control::RequestData { stream_id, window })
}

fn spawn_control_reader(
    control: mpsc::Sender<Control>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut saw_version = false;

        let result = (|| -> Result<()> {
            loop {
                let Some(packet) = read_packet(&mut stdin)? else {
                    break Ok(());
                };
                let packet_type = packet[0];
                match packet_type {
                    PACKET_VERSION => {
                        parse_version(&packet)?;
                        saw_version = true;
                    }
                    PACKET_REQUEST_DATA if saw_version => {
                        if control.send(parse_request_data(&packet)?).is_err() {
                            break Ok(());
                        }
                    }
                    PACKET_REQUEST_DATA => bail!("RequestData received before Version"),
                    other => bail!("unsupported packet type {other}"),
                }
            }
        })();

        if let Err(e) = result {
            eprintln!("protocol input error: {e}");
        }
        cancel.cancel();
    })
}

fn run(opt: Opt) -> Result<()> {
    if opt.packet_bytes < 2 {
        bail!("--packet-bytes must be at least 2");
    }

    let samp_rate = 250_000;
    let samp_rate_2 = 50_000;
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    write_version(&mut stdout)?;

    let (control_tx, control_rx) = mpsc::channel();
    let mut g = Graph::new();
    let _control_thread = spawn_control_reader(control_tx, g.cancel_token());
    let prev = blockchain![
        g,
        prev,
        RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?,
        RtlSdrDecode::new(prev),
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                samp_rate as f32,
                40_000.0,
                1_000.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::builder()
            .deci(samp_rate as usize)
            .interp(samp_rate_2 as usize)
            .build(prev)?,
        RtlSdrEncode::new(prev),
    ];
    g.add(Box::new(DataStreamSink::new(
        prev,
        stdout,
        control_rx,
        opt.stream_id,
        opt.packet_bytes,
    )));
    Ok(g.run()?)
}

fn main() -> Result<()> {
    eprintln!("rtl_data_stream receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    run(opt)
}
