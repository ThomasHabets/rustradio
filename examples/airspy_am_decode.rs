use anyhow::Result;
use clap::Parser;
use log::warn;

use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::{Complex, Float, blockchain};
use rustradio::{parse_frequency, parse_verbosity};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Input file in airspy format (I/Q s16)
    #[arg(short)]
    input: String,

    /// Verbosity level.
    #[arg(short, value_parser=parse_verbosity, default_value="info")]
    verbose: usize,

    /// Multiply audio by this before sending to sound card.
    #[arg(long = "volume", default_value = "0.1")]
    volume: Float,

    /// Sample rate.
    #[arg(long, value_parser=parse_frequency, default_value="2.5M")]
    sample_rate: f64,

    /// Audio sample rate.
    #[arg(long, value_parser=parse_frequency, default_value="48k")]
    audio_rate: f64,
}

pub fn main() -> Result<()> {
    println!("airspy am decode");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = MTGraph::new();

    let prev = blockchain![
        g,
        prev,
        FileSource::builder(&opt.input)
            // .repeat(rustradio::Repeat::infinite())
            .build()?,
        Map::keep_tags(prev, "AirSPY to Complex", |v: u32| {
            let i = (v & 0xffff) as u16 as i16;
            let q = ((v >> 16) & 0xffff) as u16 as i16;
            Complex::new(i as Float, q as Float) / 1000.0
        }),
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                opt.sample_rate as Float,
                12_500.0,
                10_000.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        Map::keep_tags(prev, "AM decode", |v| v.norm()),
        FftFilterFloat::new(
            prev,
            &rustradio::fir::low_pass(
                opt.sample_rate as Float,
                opt.audio_rate as Float,
                500.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::builder()
            .deci(opt.sample_rate as usize)
            .interp(opt.audio_rate as usize)
            .build(prev)?,
        MultiplyConst::new(prev, opt.volume),
    ];

    if true {
        g.add(Box::new(AudioSink::new(prev, opt.audio_rate as u64)?));
    } else {
        g.add(Box::new(FileSink::new(
            prev,
            "out.f32",
            rustradio::file_sink::Mode::Overwrite,
        )?));
    }

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    eprintln!("Running loop");
    g.run()?;
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}
