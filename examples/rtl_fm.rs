/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use anyhow::Result;
use log::warn;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::{Complex, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    /// Read capture file instead of live from RTL SDR.
    #[structopt(short = "r")]
    filename: Option<String>,

    /// Output file. If unset, use sound card for output.
    #[structopt(short = "o")]
    output: Option<std::path::PathBuf>,

    /// Tuned frequency, if reading from RTL SDR.
    #[allow(dead_code)]
    #[structopt(long = "freq", default_value = "100000000")]
    freq: u64,

    /// Input gain, if reading from RTL SDR.
    #[allow(dead_code)]
    #[structopt(long = "gain", default_value = "20")]
    gain: i32,

    /// Verbosity of debug messages.
    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    /// Audio volume.
    #[structopt(long = "volume", default_value = "1.0")]
    volume: Float,
}

macro_rules! blehbleh {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
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
    let samp_rate = 1_024_000.0;

    let prev = if let Some(filename) = opt.filename {
        blehbleh!(g, FileSource::<Complex>::new(&filename, false)?)
    } else if !cfg!(feature = "rtlsdr") {
        panic!("RTL SDR feature not enabled")
    } else {
        // RTL SDR source.
        #[cfg(feature = "rtlsdr")]
        {
            let src = Box::new(RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?);
            let dec = Box::new(RtlSdrDecode::new(src.out()));
            let prev = dec.out();
            g.add(src);
            g.add(dec);
            prev
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("can't happen, but must be here to compile")
    };

    // Filter.
    let taps = rustradio::fir::low_pass_complex(
        samp_rate,
        100_000.0,
        1000.0,
        &rustradio::window::WindowType::Hamming,
    );
    let prev = blehbleh![g, FftFilter::new(prev, &taps)];

    // Resample.
    let new_samp_rate = 200_000.0;
    let prev = blehbleh![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    // TODO: Add broadcast FM deemph.

    // Quad demod.
    let prev = blehbleh![g, QuadratureDemod::new(prev, 1.0)];

    let taps = rustradio::fir::low_pass(
        samp_rate,
        44_100.0,
        500.0,
        &rustradio::window::WindowType::Hamming,
    );
    //let audio_filter = FIRFilter::new(prev, &taps);
    let prev = blehbleh![g, FftFilterFloat::new(prev, &taps)];

    // Resample audio.
    let new_samp_rate = 48_000.0;
    let prev = blehbleh![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];

    // Change volume.
    let prev = blehbleh![g, MultiplyConst::new(prev, opt.volume)];

    if let Some(out) = opt.output {
        // Convert to .au.
        let prev = blehbleh![
            g,
            AuEncode::new(
                prev,
                rustradio::au::Encoding::PCM16,
                new_samp_rate as u32,
                1
            )
        ];
        // Save to file.
        g.add(Box::new(FileSink::new(prev, out, Mode::Overwrite)?));
    } else if !cfg!(feature = "audio") {
        panic!("Rustradio build without feature 'audio'. Can only write to file with -o, not play live.");
    } else {
        #[cfg(feature = "audio")]
        {
            // Play live.
            g.add(Box::new(AudioSink::new(prev, new_samp_rate as u64)));
        }
    }

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    let st = std::time::Instant::now();
    eprintln!("Running loop");
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
