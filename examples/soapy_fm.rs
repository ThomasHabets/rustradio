/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use anyhow::Result;
use log::warn;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::Float;

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "d")]
    driver: String,

    #[structopt(short = "o")]
    output: String,

    // Unused if soapysdr feature not enabled.
    #[allow(dead_code)]
    #[structopt(long = "freq", default_value = "100000000")]
    freq: u64,

    // Unused if soapysdr feature not enabled.
    #[allow(dead_code)]
    #[structopt(long = "gain", default_value = "20")]
    gain: i32,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

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
    println!("soapy_fm receiver example");
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = Graph::new();
    let samp_rate = 1_024_000.0f32;

    let prev = {
        #[cfg(feature = "soapysdr")]
        {
            blehbleh![
                g,
                SoapySdrSourceBuilder::new(opt.driver.clone(), opt.freq as f64, samp_rate as f64)
                    .igain(opt.gain as f64)
                    .build()?
            ]
        }
        #[cfg(not(feature = "soapysdr"))]
        panic!("can't happen, but must be here to compile")
    };

    // Filter.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 100_000.0, 1000.0);
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

    let taps = rustradio::fir::low_pass(samp_rate, 44_100.0, 500.0);
    //let audio_filter = FIRFilter::new(prev, &taps);
    let prev = blehbleh![g, FftFilterFloat::new(prev, &taps)];

    // Resample audio.
    let new_samp_rate = 48_000.0;
    let prev = blehbleh![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let _samp_rate = new_samp_rate;

    // Change volume.
    let prev = blehbleh![g, MultiplyConst::new(prev, opt.volume)];

    // Convert to .au.
    let prev = blehbleh![
        g,
        AuEncode::new(prev, rustradio::au::Encoding::PCM16, 48000, 1)
    ];

    // Save to file.
    g.add(Box::new(FileSink::new(prev, &opt.output, Mode::Overwrite)?));

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
