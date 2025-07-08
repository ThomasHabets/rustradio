//! Bell 202 modem. Most common used modem for AX.25.
//!
//! This example got its own blog post. See
//! <https://blog.habets.se/2025/06/Software-defined-KISS-modem.html>.
//!
//! ## Examples
//!
//! ### Send/receive APRS
//!
//! 1. Run the modem: `bell202 --freq 144.8m -d driver=uhd --ogain=0.5`
//! 2. Connect to port 7878 using some app that can talk APRS using KISS.
//!
//! ### Remote SoapySDR, connect to kernel AX.25
//!
//! 1. On the machine with the SDR, run `SoapySDRServer --bind`
//! 2. Start bell202: `bell202 --freq 433.8m -d soapy=0,remote=hostname-here,driver=remote,remote:driver=uhd --ogain 0.5 --tty ./convenience.symlink`
//! 3. Connect to kernel: `kissattach ./convenience.symlink radioname` (set up radioname in /etc/ax25/axports)
//! 4. Disable CRC on kernel KISS: `kissparms -c 1 -p radioname`
//!
//! Kernel stack should now be up and working with bell202 as the modem.
use std::io::{Read, Write};

use anyhow::Result;
use clap::Parser;
use log::info;

use rustradio::Float;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::{parse_frequency, parse_verbosity};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// SoapySDR driver string.
    #[arg(short)]
    driver: String,

    /// Verbosity level.
    #[arg(short, value_parser=parse_verbosity, default_value = "info")]
    verbose: usize,

    /// TX/RX frequency.
    #[arg(long, value_parser=parse_frequency)]
    freq: f64,

    #[arg(long, value_parser=parse_frequency, default_value_t = 300000.0)]
    sample_rate: f64,

    /// Output gain. 0.0-1.0.
    #[arg(long, default_value_t = 0.0)]
    ogain: f64,

    #[arg(long, default_value = "0.5")]
    symbol_max_deviation: Float,

    #[arg(
        long = "symbol_taps",
        default_value = "0.5,0.5",
        use_value_delimiter = true
    )]
    symbol_taps: Vec<Float>,

    /// Listen for KISS on this address.
    #[arg(long)]
    tcp_listen: Option<String>,

    /// Listen for KISS on a tty. Create symlink to the new TTY for this path.
    #[cfg(feature = "nix")]
    #[arg(long)]
    tty: Option<std::path::PathBuf>,
}

// Get a reader and a writer where we'll be talking KISS.
fn get_kiss_stream(opt: &Opt) -> Result<(Box<dyn Write + Send>, Box<dyn Read + Send>)> {
    if let Some(addr) = &opt.tcp_listen {
        let listener = std::net::TcpListener::bind(addr)?;
        info!("Awaiting connection");
        let (tcp, addr) = listener.accept()?;
        drop(listener);
        info!("Connect from {addr}");
        info!("Setting up device");
        let rx = tcp.try_clone()?;
        return Ok((Box::new(tcp), Box::new(rx)));
    }
    #[cfg(feature = "nix")]
    {
        if opt.tcp_listen.is_some() && opt.tty.is_some() {
            return Err(anyhow::Error::msg("-tcp and -tty are mutually exclusive"));
        }
        use nix::pty::{OpenptyResult, openpty};
        use std::ffi::CStr;
        use std::fs::File;
        use std::os::fd::AsRawFd;
        if let Some(path) = &opt.tty {
            let OpenptyResult { master, slave } = openpty(None, None)?;
            // SAFETY:
            // Input is from a successful openpty().
            let ptr = unsafe { libc::ptsname(master.as_raw_fd()) };
            if ptr.is_null() {
                return Err(anyhow::Error::msg(
                    "ptsname() on newly created TTY returned NULL",
                ));
            }
            // SAFETY:
            // We have checked for null above.
            let slave_name = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
            info!("Slave tty device: {slave_name}");
            if let Err(e) = std::os::unix::fs::symlink(&slave_name, path) {
                if e.kind() != std::io::ErrorKind::AlreadyExists
                    || !path.symlink_metadata()?.is_symlink()
                {
                    Err(rustradio::Error::file_io(e, path))?;
                }
                log::trace!(
                    "Symlink {} already exists. Attempting to delete it",
                    path.display()
                );
                std::fs::remove_file(path).map_err(|e| rustradio::Error::file_io(e, path))?;
                std::os::unix::fs::symlink(slave_name, path)
                    .map_err(|e| rustradio::Error::file_io(e, path))?;
            }
            let rx = master.try_clone()?;
            std::mem::forget(slave);
            return Ok((Box::new(File::from(master)), Box::new(File::from(rx))));
        }
    }
    Err(anyhow::Error::msg("-tcp or -tty is mandatory"))
}

pub fn main() -> Result<()> {
    println!("bell202 modem");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .module("soapysdr")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    soapysdr::configure_logging();

    let mut g = MTGraph::new();

    let (kiss, rx) = get_kiss_stream(&opt)?;
    let dev = soapysdr::Device::new(&*opt.driver)?;

    // Transmitter.
    {
        info!("Setting up transmitter");
        let cancel = g.cancel_token();
        let baud = 1200.0;
        let audio_rate = 48000.0;
        let prev = blockchain![
            g,
            prev,
            ReaderSource::new(rx)?,
            KissFrame::new(prev),
            KissDecode::new(prev),
            FcsAdder::new(prev),
            HdlcFramer::new(prev),
            PduToStream::new(prev),
            NrziEncode::new(prev),
            RationalResampler::builder()
                .deci(baud as usize)
                .interp(audio_rate as usize)
                .build(prev)?,
            Map::keep_tags(prev, "bits_to_pn", |s| if s > 0 {
                2200.0 as Float
            } else {
                1200.0
            }),
            Vco::new(prev, 2.0 * std::f64::consts::PI / audio_rate),
            Map::keep_tags(prev, "ComplexToFloat", |s| s.re),
            MultiplyConst::new(prev, 0.5),
            RationalResampler::builder()
                .deci(audio_rate as usize)
                .interp(opt.sample_rate as usize)
                .build(prev)?,
            Vco::new(prev, 2.0 * std::f64::consts::PI * 5000.0 / opt.sample_rate),
            Canary::new(prev, move || cancel.cancel()),
        ];
        g.add(Box::new(
            SoapySdrSink::builder(&dev, opt.freq, opt.sample_rate)
                .ogain(opt.ogain)
                .build(prev)?,
        ));
    }

    // Receiver.
    {
        info!("Setting up receiver");
        let samp_rate = 300_000.0f32;
        let samp_rate_2 = 50_000.0;
        let freq1 = 1200.0;
        let freq2 = 2200.0f32;
        let center_freq = freq1 + (freq2 - freq1) / 2.0;
        let baud = 1200.0;
        let prev = blockchain![
            g,
            prev,
            SoapySdrSource::builder(&dev, opt.freq, samp_rate as f64).build()?,
            FftFilter::new(
                prev,
                rustradio::fir::low_pass_complex(
                    samp_rate,
                    20_000.0,
                    100.0,
                    &rustradio::window::WindowType::Hamming,
                )
            ),
            RationalResampler::builder()
                .deci(samp_rate as usize)
                .interp(samp_rate_2 as usize)
                .build(prev)?,
            QuadratureDemod::new(prev, 1.0),
            Hilbert::new(prev, 65, &rustradio::window::WindowType::Hamming),
            QuadratureDemod::new(prev, 1.0),
            FftFilterFloat::new(
                prev,
                &rustradio::fir::low_pass(
                    samp_rate_2,
                    1100.0,
                    100.0,
                    &rustradio::window::WindowType::Hamming,
                )
            ),
            add_const(
                prev,
                -center_freq * 2.0 * std::f32::consts::PI / samp_rate_2
            ),
            SymbolSync::new(
                prev,
                samp_rate_2 / baud,
                opt.symbol_max_deviation,
                Box::new(rustradio::symbol_sync::TedZeroCrossing::new()),
                Box::new(rustradio::iir_filter::IirFilter::new(&opt.symbol_taps)),
            ),
            BinarySlicer::new(prev),
            NrziDecode::new(prev),
            HdlcDeframer::new(prev, 10, 1500),
            KissEncode::new(prev),
            PduToStream::new(prev),
        ];
        g.add(Box::new(WriterSink::new(prev, kiss)));
    }
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");
    g.run()?;
    println!("{}", g.generate_stats().expect("failed to generate stats"));
    Ok(())
}
