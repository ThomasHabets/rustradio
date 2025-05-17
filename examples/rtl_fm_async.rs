/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use anyhow::Result;
use clap::Parser;
use log::warn;

use rustradio::agraph::AsyncGraph;
use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::GraphRunner;
use rustradio::{Complex, Float, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Read capture file instead of live from RTL SDR.
    #[arg(short)]
    filename: Option<String>,

    /// Loop the read file forever.
    #[arg(long)]
    file_repeat: bool,

    /// Output file. If unset, use sound card for output.
    #[arg(short)]
    output: Option<std::path::PathBuf>,

    /// Tuned frequency, if reading from RTL SDR.
    #[allow(dead_code)]
    #[arg(long = "freq", default_value = "100000000")]
    freq: u64,

    /// Input gain, if reading from RTL SDR.
    #[allow(dead_code)]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,

    /// Verbosity of debug messages.
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// Audio volume.
    #[arg(long = "volume", default_value = "1.0")]
    volume: Float,

    /// Audio output rate.
    #[arg(default_value = "48000")]
    audio_rate: u32,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    println!("rtl_fm receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = AsyncGraph::new();
    let samp_rate = 1_024_000.0;

    let repeat = if opt.file_repeat {
        rustradio::Repeat::infinite()
    } else {
        rustradio::Repeat::finite(1)
    };
    let prev = if let Some(filename) = opt.filename {
        blockchain![
            g,
            prev,
            FileSource::<Complex>::builder(&filename)
                .repeat(repeat)
                .build()?,
        ]
    } else if !cfg!(feature = "rtlsdr") {
        panic!("RTL SDR feature not enabled")
    } else {
        // RTL SDR source.
        #[cfg(feature = "rtlsdr")]
        {
            blockchain![
                g,
                prev,
                RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?,
                RtlSdrDecode::new(prev),
            ]
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("can't happen, but must be here to compile")
    };

    let samp_rate_2 = 200_000.0;
    let audio_rate = opt.audio_rate as f32;
    let prev = blockchain![
        g,
        prev,
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                samp_rate,
                100_000.0,
                1000.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResamplerBuilder::new()
            .deci(samp_rate as usize)
            .interp(samp_rate_2 as usize)
            .build(prev)?,
        QuadratureDemod::new(prev, 1.0),
        FftFilterFloat::new(
            prev,
            &rustradio::fir::low_pass(
                samp_rate_2,
                44_100.0,
                500.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::new(prev, audio_rate as usize, samp_rate_2 as usize)?,
        // Change volume.
        MultiplyConst::new(prev, opt.volume),
    ];

    if let Some(out) = opt.output {
        // Convert to .au.
        let prev = blockchain![
            g,
            prev,
            AuEncode::new(prev, rustradio::au::Encoding::Pcm16, audio_rate as u32, 1)
        ];
        // Save to file.
        g.add(Box::new(FileSink::new(prev, out, Mode::Overwrite)?));
    } else if !cfg!(feature = "audio") {
        panic!(
            "Rustradio build without feature 'audio'. Can only write to file with -o, not play live."
        );
    } else {
        #[cfg(feature = "audio")]
        {
            // Play live.
            g.add(Box::new(AudioSink::new(prev, audio_rate as u64)?));
        }
    }

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    eprintln!("Running loop");
    g.run_async().await?;
    eprintln!("{}", g.generate_stats().unwrap_or("no stats".to_string()));
    Ok(())
}
