//! RTL-SDR source that serves downsampled data over the DATA_STREAM.md protocol.
//!
//! The transport is stdin/stdout. Control packets are read from stdin and data
//! packets are written to stdout.
use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use anyhow::{Result, bail};
use clap::Parser;

use rustradio::block::{Block, BlockEOF, BlockName, BlockRet};
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::data_stream::{DataStreamId, Packet, RequestData, SyncReader, SyncWriter};
use rustradio::graph::{CancellationToken, Graph, GraphRunner};
use rustradio::stream::ReadStream;

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
    stream_id: DataStreamId,

    /// Maximum data bytes to put in one protocol Data packet.
    #[arg(long = "packet-bytes", default_value_t = 16_384)]
    packet_bytes: usize,
}

struct DataStreamSink<W> {
    src: ReadStream<u8>,
    writer: SyncWriter<W>,
    control: Receiver<RequestData>,
    stream_id: DataStreamId,
    window: usize,
    max_packet_data: usize,
}

impl<W> DataStreamSink<W>
where
    W: Write + Send,
{
    #[must_use]
    fn new(
        src: ReadStream<u8>,
        writer: SyncWriter<W>,
        control: Receiver<RequestData>,
        stream_id: DataStreamId,
        max_packet_data: usize,
    ) -> Self {
        Self {
            src,
            writer,
            control,
            stream_id,
            window: 0,
            max_packet_data,
        }
    }

    fn update_window(&mut self) {
        while let Ok(req) = self.control.try_recv() {
            if req.stream_id == self.stream_id {
                self.window = req.window;
            }
        }
    }

    fn write_data_packet(&mut self, data: &[u8]) -> rustradio::Result<()> {
        self.writer.write_data(&self.stream_id, data)
    }
}

impl<W> BlockName for DataStreamSink<W> {
    fn block_name(&self) -> &str {
        "DataStreamSink"
    }
}

impl<W> BlockEOF for DataStreamSink<W> {
    fn eof(&mut self) -> bool {
        self.src.eof()
    }
}

impl<W> Block for DataStreamSink<W>
where
    W: Write + Send,
{
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

fn spawn_control_reader(
    control: mpsc::Sender<RequestData>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let stdin = std::io::stdin().lock();
        let mut reader = SyncReader::new(stdin);

        let result = (|| -> Result<()> {
            if !reader.read_version()? {
                return Ok(());
            }
            loop {
                let Some(packet) = reader.read_packet()? else {
                    break Ok(());
                };
                match packet {
                    Packet::RequestData(req) => {
                        if control.send(req).is_err() {
                            return Ok(());
                        };
                    }
                    other => bail!("unexpected protocol input packet: {other:?}"),
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
    let stdout = std::io::BufWriter::new(std::io::stdout());
    let mut writer = SyncWriter::new(stdout);
    writer.write_version()?;

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
        writer,
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
