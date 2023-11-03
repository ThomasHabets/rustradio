/*! Test program for decoding G3RUH 9600bps AX.25 using whole packet
clock recovery.
*/
use std::path::PathBuf;

use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::{Error, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "r")]
    read: Option<String>,

    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "freq", default_value = "144800000")]
    freq: u64,

    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "gain", default_value = "20")]
    gain: i32,

    #[structopt(long = "rtlsdr")]
    rtlsdr: bool,

    #[structopt(long = "sample_rate", short = "s", default_value = "50000")]
    samp_rate: Float,

    #[structopt(long = "out", short = "o")]
    output: PathBuf,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    #[structopt(long = "threshold", default_value = "0.0001")]
    threshold: Float,

    #[structopt(long = "iir_alpha", default_value = "0.01")]
    iir_alpha: Float,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()
        .unwrap();

    let samp_rate = opt.samp_rate;
    let mut g = Graph::new();

    // Source.
    //let prev = add_block![g, FileSource::new(&opt.read, false)?];
    let prev = if let Some(read) = opt.read {
        add_block![g, FileSource::new(&read, false)?]
    } else if opt.rtlsdr {
        #[cfg(feature = "rtlsdr")]
        {
            // Source.
            let prev = add_block![g, RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?];

            // Decode.
            add_block![g, RtlSdrDecode::new(prev)]
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("rtlsdr feature not enabled")
    } else {
        panic!("Need to provide either --rtlsdr or -r")
    };

    // Filter.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 20_000.0, 100.0);
    let prev = add_block![g, FftFilter::new(prev, &taps)];

    // Resample RF.
    let new_samp_rate = 50_000.0;
    let prev = add_block![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    // Tee out signal strength.
    let (prev, burst_tee) = add_block![g, Tee::new(prev)];
    let burst_tee = add_block![g, ComplexToMag2::new(burst_tee)];
    let burst_tee = add_block![
        g,
        SinglePoleIIRFilter::new(burst_tee, opt.iir_alpha)
            .ok_or(Error::new("bad IIR parameters"))?
    ];

    // Demod.
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    // Filter.
    let taps = rustradio::fir::low_pass(samp_rate, 16000.0, 100.0);
    let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    // Tag burst.
    let prev = add_block![
        g,
        BurstTagger::new(prev, burst_tee, opt.threshold, "burst".to_string())
    ];

    // Create quad demod raw sample blobs (Vec<Float>) from tagged
    // stream of Floats.
    let prev = add_block![
        g,
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, 50)
    ];

    // A kind of frequency lock.
    let prev = add_block![g, Midpointer::new(prev)];

    // Symbol sync.
    let prev = add_block![g, WpcrBuilder::new(prev).samp_rate(samp_rate).build()];

    // Turn Vec<Float> into Float.
    let prev = add_block![g, VecToStream::new(prev)];

    // Turn floats into bits.
    let prev = add_block![g, BinarySlicer::new(prev)];

    // NRZI decode.
    let prev = add_block![g, NrziDecode::new(prev)];

    // G3RUH descramble.
    let prev = add_block![g, Descrambler::new(prev, 0x21, 0, 16)];

    // Decode.
    let prev = add_block![g, HdlcDeframer::new(prev, 10, 1500)];

    // Save.
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Run.
    let st = std::time::Instant::now();
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-9600-rx -- -r ../aprs-9600-50k.c32 -o ../packets"
 * End:
 */
