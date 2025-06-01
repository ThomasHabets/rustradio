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
        let test_packet = vec![
            0x82u8, 0xa0, 0x96, 0x60, 0x60, 0x68, 0x60, 0x9a, 0x60, 0xa8, 0x90, 0x86, 0x40, 0xea,
            0xae, 0x92, 0x88, 0x8a, 0x62, 0x40, 0x62, 0xae, 0x92, 0x88, 0x8a, 0x64, 0x40, 0x63,
            0x03, 0xf0, 0x3b, 0x47, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x20, 0x20, 0x20, 0x2a, 0x37,
            0x31, 0x33, 0x32, 0x39, 0x30, 0x7a, 0x35, 0x31, 0x33, 0x31, 0x2e, 0x39, 0x39, 0x4e,
            0x2f, 0x30, 0x30, 0x30, 0x30, 0x37, 0x2e, 0x35, 0x35, 0x57, 0x2e, 0x47, 0x6f, 0x6f,
            0x67, 0x6c, 0x65, 0x0d,
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
