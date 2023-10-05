/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::stream::StreamType;
use rustradio::{Complex, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "r")]
    filename: Option<String>,

    #[structopt(short = "o")]
    output: String,

    // Unused if rtlsdr feature not enabled.
    #[allow(dead_code)]
    #[structopt(long = "freq", default_value = "100000000")]
    freq: u64,

    // Unused if rtlsdr feature not enabled.
    #[allow(dead_code)]
    #[structopt(long = "gain", default_value = "20")]
    gain: i32,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    #[structopt(long = "no-audio-filter")]
    no_audio_filter: bool,

    #[structopt(long = "volume", default_value = "1.0")]
    volume: Float,
}

fn main() -> Result<()> {
    println!("rtl_fm receiver example");
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = Graph::new();
    let samp_rate = 1024_000.0;

    let prev = if let Some(filename) = opt.filename {
        g.add(Box::new(FileSource::<Complex>::new(&filename, false)?))
    } else {
        if !cfg!(feature = "rtlsdr") {
            panic!("RTL SDR feature not enabled")
        } else {
            // RTL SDR source.
            #[cfg(feature = "rtlsdr")]
            {
                let src = g.add(Box::new(RtlSdrSource::new(
                    opt.freq,
                    samp_rate as u32,
                    opt.gain,
                )?));
                let dec = g.add(Box::new(RtlSdrDecode::new()));
                g.connect(StreamType::new_u8(), src, 0, dec, 0);
                dec
            }
            #[cfg(not(feature = "rtlsdr"))]
            panic!("can't happen")
        }
    };

    // Filter.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 100_000.0, 1000.0);
    let filter = g.add(Box::new(FftFilter::new(&taps)));
    g.connect(StreamType::new_complex(), prev, 0, filter, 0);

    // Resample.
    let new_samp_rate = 200_000.0;
    let resamp = g.add(Box::new(RationalResampler::new(
        new_samp_rate as usize,
        samp_rate as usize,
    )?));
    g.connect(StreamType::new_complex(), filter, 0, resamp, 0);
    let samp_rate = new_samp_rate;

    // TODO: Add broadcast FM deemph.

    // Quad demod.
    let quad = g.add(Box::new(QuadratureDemod::new(1.0)));
    g.connect(StreamType::new_complex(), resamp, 0, quad, 0);

    let prev = if !opt.no_audio_filter {
        // Audio filter.
        let taps = rustradio::fir::low_pass(samp_rate, 44_100.0, 500.0);
        let audio_filter = g.add(Box::new(FIRFilter::new(&taps)));
        g.connect(StreamType::new_float(), quad, 0, audio_filter, 0);
        audio_filter
    } else {
        quad
    };

    // Resample audio.
    let new_samp_rate = 48_000.0;
    let audio_resamp = g.add(Box::new(RationalResampler::new(
        new_samp_rate as usize,
        samp_rate as usize,
    )?));
    let _samp_rate = new_samp_rate;
    g.connect(StreamType::new_float(), prev, 0, audio_resamp, 0);

    // Change volume.
    let volume = g.add(Box::new(MultiplyConst::new(opt.volume)));
    g.connect(StreamType::new_float(), audio_resamp, 0, volume, 0);

    // Convert to .au.
    let au = g.add(Box::new(AuEncode::new(
        rustradio::au::Encoding::PCM16,
        48000,
        1,
    )));
    g.connect(StreamType::new_float(), volume, 0, au, 0);

    // Save to file.
    let sink = g.add(Box::new(FileSink::<u8>::new(&opt.output, Mode::Overwrite)?));
    g.connect(StreamType::new_u8(), au, 0, sink, 0);

    g.run()
}
