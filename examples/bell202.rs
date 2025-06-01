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

    // Transmitter.
    let dev = soapysdr::Device::new(&*opt.driver)?;
    {
        eprintln!("Set up transmitter");
        // Message including CRC: WORKS!!!!111
        let test_packet = vec![
            0x82, 0xa0, 0xb4, 0x60, 0x60, 0x62, 0x60, 0x9a, 0x60, 0xa8, 0x90, 0x86, 0x40, 0xe5,
            0x03, 0xf0, 0x3a, 0x4d, 0x30, 0x54, 0x48, 0x43, 0x2d, 0x31, 0x20, 0x20, 0x3a, 0x68,
            0x65, 0x6c, 0x6c, 0x6f, 0x41, 0x7d, 0xdc,
        ];
        /*
        let (tx, rx) = rustradio::stream::new_nocopy_stream();
        tx.push(test_packet, &[]);
        */
        let baud = 1200.0;
        let audio_rate = 48000.0;
        let prev = blockchain![
            g,
            prev,
            Strobe::new(std::time::Duration::from_millis(2000), test_packet),
            FcsAdder::new(prev),
            HdlcFramer::new(prev),
            PduToStream::new(prev),
            Map::keep_tags(prev, "bool_to_u8", |s| if s { 1 } else { 0 }),
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
        g.add(Box::new(NullSink::new(prev)));
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
