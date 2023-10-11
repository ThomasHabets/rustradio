/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use anyhow::Result;
use log::warn;
use structopt::StructOpt;

use rustradio::block::Block;
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

    let mut v: Vec<&mut dyn Block> = Vec::new();

    //let (t, prev) = if let Some(filename) = opt.filename {
    let mut src = FileSource::<Complex>::new(&opt.filename.unwrap(), false)?;
    let prev = src.out();
    v.push(&mut src);

    //} else {
    /*        if !cfg!(feature = "rtlsdr") {
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
    }*/
    //};

    // Filter.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 100_000.0, 1000.0);
    let mut filter = FftFilter::new(prev, &taps);
    let prev = filter.out();
    v.push(&mut filter);

    // Resample.
    let new_samp_rate = 200_000.0;
    let mut resamp = RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?;
    let prev = resamp.out();
    v.push(&mut resamp);
    let samp_rate = new_samp_rate;

    // TODO: Add broadcast FM deemph.

    // Quad demod.
    let mut quad = QuadratureDemod::new(prev, 1.0);
    let prev = quad.out();
    v.push(&mut quad);

    /*
    let prev = if !opt.no_audio_filter {
        // Audio filter.
        let taps = rustradio::fir::low_pass(samp_rate, 44_100.0, 500.0);
        //let audio_filter = g.add(Box::new(FIRFilter::new(&taps)));
        let audio_filter = g.add(Box::new(FftFilterFloat::new(&taps)));
        g.connect(StreamType::new_float(), quad, 0, audio_filter, 0);
        audio_filter
    } else {
        quad
    };
     */

    // Resample audio.
    let new_samp_rate = 48_000.0;
    let mut audio_resamp =
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?;
    let prev = audio_resamp.out();
    v.push(&mut audio_resamp);
    let _samp_rate = new_samp_rate;

    // Change volume.
    let mut volume = MultiplyConst::new(prev, opt.volume);
    let prev = volume.out();
    v.push(&mut volume);

    // Convert to .au.
    let mut au = AuEncode::new(prev, rustradio::au::Encoding::PCM16, 48000, 1);
    let prev = au.out();
    v.push(&mut au);

    // Save to file.
    let mut sink = FileSink::new(prev, &opt.output, Mode::Overwrite)?;
    v.push(&mut sink);
    /*
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    let st = std::time::Instant::now();
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
     */
    eprintln!("Running loop");
    loop {
        for b in &mut v {
            //eprintln!("  Running block {}", b.block_name());
            b.work()?;
        }
    }
    Ok(())
}
