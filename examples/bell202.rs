use anyhow::Result;
use clap::Parser;

use rustradio::Float;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    driver: String,
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// TX/RX frequency.
    #[arg(long)]
    freq: f64,

    #[arg(long, default_value_t = 300000.0)]
    sample_rate: f64,

    /// Output gain. 0.0-1.0.
    #[arg(long, default_value_t = 0.0)]
    ogain: f64,
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
        let prev = blockchain![
            g,
            prev,
            SoapySdrSource::builder(&dev, 2_450_000_000.0, 300000.0).build()?
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
