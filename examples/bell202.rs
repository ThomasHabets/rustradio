//! Bell 202 modem. Most common used modem for AX.25.
//!
//! ## Examples
//!
//! ### Send/receive APRS
//!
//! 1. Run the modem: `bell202 --freq 433800000 -d driver=uhd --ogain=0.5`
//! 2. Connect to port 7878 using some app that can talk APRS using KISS. :-)
//!
//! ### Remote SoapySDR, connect to kernel AX.25
//!
//! 1. On the machine with the SDR, run `SoapySDRServer --bind`
//! 2. Start bell202: `bell202 --freq 433800000 -d soapy=0,remote=hostname-here,driver=remote,remote:driver=uhd --ogain 0.5`
//! 3. Create a TCP-tty bridge: `socat -d -d PTY,raw,echo=0 TCP:localhost:7878`
//! 4. Connect to kernel: `kissattach /dev/tty/<tty from prev command>
//!    radioname` (set up radioname in /etc/ax25/axports)
//! 5. Disable CRC on kernel KISS: `kissparms -c 1 -p radioname`
//!
//! Kernel stack should now be up and working with bell202 as the modem.
use anyhow::Result;
use clap::Parser;

use rustradio::Float;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::parse_frequency;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    driver: String,
    #[arg(short, default_value = "0")]
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
}

pub fn main() -> Result<()> {
    println!("soapy_fm receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = MTGraph::new();

    let listener = std::net::TcpListener::bind("[::]:7878")?;
    println!("Awaiting connection");
    let (tcp, addr) = listener.accept()?;
    drop(listener);
    println!("Connect from {addr}");

    // Transmitter.
    let dev = soapysdr::Device::new(&*opt.driver)?;
    {
        eprintln!("Set up transmitter");
        let baud = 1200.0;
        let audio_rate = 48000.0;
        let prev = blockchain![
            g,
            prev,
            ReaderSource::new(tcp.try_clone()?),
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
        ];
        g.add(Box::new(
            SoapySdrSink::builder(&dev, opt.freq, opt.sample_rate)
                .ogain(opt.ogain)
                .build(prev)?,
        ));
    }

    // Receiver.
    {
        eprintln!("Set up receiver");
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
        // TODO: decode.
        g.add(Box::new(WriterSink::new(prev, tcp)));
    }
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");
    g.run()?;
    eprintln!("{}", g.generate_stats().expect("failed to generate stats"));
    Ok(())
}
