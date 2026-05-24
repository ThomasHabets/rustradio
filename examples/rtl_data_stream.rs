//! RTL-SDR source that serves downsampled data over the DATA_STREAM.md protocol.
//!
//! The transport is stdin/stdout. Control packets are read from stdin and data
//! packets are written to stdout.
use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use anyhow::{Result, bail};
use clap::Parser;

use rustradio::Error;
use rustradio::block::{Block, BlockRet};
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

    /// Sample rate.
    #[arg(long, short, default_value_t = 250_000)]
    sample_rate: u32,

    /// Sample rate.
    #[arg(long, short, default_value_t = 50_000)]
    downsample_rate: u32,

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

/// A sink that writes data to a DataStream, probably a websocket heading for
/// the UI.
#[derive(rustradio_macros::Block)]
#[rustradio(new)]
struct DataStreamSink<W> {
    #[rustradio(in)]
    src: ReadStream<u8>,
    writer: SyncWriter<W>,
    stream_id: DataStreamId,
    max_packet_data: usize,

    #[rustradio(default)]
    control: Option<Receiver<RequestData>>,
    #[rustradio(default)]
    window: usize,
}

impl<W> DataStreamSink<W>
where
    W: Write + Send,
{
    /// Get control channel.
    pub fn control(&mut self) -> Result<mpsc::Sender<RequestData>> {
        if self.control.is_some() {
            return Err(Error::msg("DataStreamSink::control called twice").into());
        }
        let (tx, rx) = mpsc::channel();
        self.control = Some(rx);
        Ok(tx)
    }

    // Check for a new window size.
    fn update_window(&mut self) {
        let Some(ref control) = self.control else {
            return;
        };
        while let Ok(req) = control.try_recv() {
            if req.stream_id == self.stream_id {
                self.window = req.window;
            }
        }
    }

    // Send requested data on the connection.
    fn write_data_packet(&mut self, data: &[u8]) -> rustradio::Result<()> {
        self.writer.write_data(&self.stream_id, data)
    }
}

impl<W> Block for DataStreamSink<W>
where
    W: Write + Send,
{
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        loop {
            // First clear any pending control messages.
            self.update_window();

            // Then check for input, since input can be slept on.
            let (input, _tags) = self.src.read_buf()?;
            if input.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 2));
            }

            // Window updates can currently not be waited for. TODO: is there a
            // way we could make it pending on the control channel?
            if self.window == 0 {
                return Ok(BlockRet::Pending);
            }

            let n = input.len().min(self.window).min(self.max_packet_data);
            debug_assert_ne!(
                n, 0,
                "this should not be possible. We just checked it! Unless max packet data ({}) is 0?",
                self.max_packet_data
            );
            if n == 0 {
                return Ok(BlockRet::Pending);
            }

            self.write_data_packet(&input.slice()[..n])?;
            input.consume(n);
            self.window -= n;
        }
    }
}

// Reader side of the websocket reader.
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

    let samp_rate = opt.sample_rate;
    let samp_rate_2 = opt.downsample_rate;
    let stdout = std::io::BufWriter::new(std::io::stdout());
    let mut writer = SyncWriter::new(stdout);
    writer.write_version()?;

    let mut g = Graph::new();
    let prev = blockchain![
        g,
        prev,
        RtlSdrSource::new(opt.freq, samp_rate, opt.gain)?,
        RtlSdrDecode::new(prev),
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                samp_rate as f32,
                (opt.downsample_rate as f32) * 0.8,
                1_000.0, // Sharp filter.
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::builder()
            .deci(samp_rate as usize)
            .interp(samp_rate_2 as usize)
            .build(prev)?,
        RtlSdrEncode::new(prev),
    ];
    let mut sink = DataStreamSink::new(prev, writer, opt.stream_id, opt.packet_bytes);
    let control_tx = sink.control()?;
    g.add(Box::new(sink));
    let _control_thread = spawn_control_reader(control_tx, g.cancel_token());
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
