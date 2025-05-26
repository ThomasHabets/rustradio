/*! Test program for decoding G3RUH 9600bps AX.25 using whole packet
clock recovery.
*/
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::{Error, Float, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    read: Option<String>,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "freq", default_value = "144800000")]
    freq: u64,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,

    #[arg(long = "rtlsdr")]
    rtlsdr: bool,

    #[arg(long = "sample_rate", short, default_value = "50000")]
    samp_rate: Float,

    #[arg(long = "out", short)]
    output: PathBuf,

    #[arg(short, default_value = "0")]
    verbose: usize,

    #[arg(long = "threshold", default_value = "0.0001")]
    threshold: Float,

    #[arg(long = "iir_alpha", default_value = "0.01")]
    iir_alpha: Float,
}

fn main() -> Result<()> {
    let opt = Opt::parse();
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
        blockchain![g, prev, FileSource::new(&read)?]
    } else if opt.rtlsdr {
        #[cfg(feature = "rtlsdr")]
        {
            // Source.
            blockchain![
                g,
                prev,
                RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?,
                RtlSdrDecode::new(prev),
            ]
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("rtlsdr feature not enabled")
    } else {
        panic!("Need to provide either --rtlsdr or -r")
    };

    // Filter.
    let taps = rustradio::fir::low_pass_complex(
        samp_rate,
        20_000.0,
        100.0,
        &rustradio::window::WindowType::Hamming,
    );
    let new_samp_rate = 50_000.0;
    let prev = blockchain![
        g,
        prev,
        FftFilter::new(prev, taps),
        // Resample RF.
        RationalResampler::builder()
            .deci(samp_rate as usize)
            .interp(new_samp_rate as usize)
            .build(prev)?,
    ];
    let samp_rate = new_samp_rate;

    // Tee out signal strength.
    let (b, prev, burst_tee) = Tee::new(prev);
    g.add(Box::new(b));
    let burst_tee = blockchain![
        g,
        burst_tee,
        ComplexToMag2::new(burst_tee),
        SinglePoleIirFilter::new(burst_tee, opt.iir_alpha)
            .ok_or(Error::msg("bad IIR parameters"))?,
    ];

    // Demod.
    let prev = blockchain![
        g,
        prev,
        QuadratureDemod::new(prev, 1.0),
        // Filter.
        //FftFilterFloat::new(prev, &rustradio::fir::low_pass(samp_rate, 16000.0, 100.0));
        // Tag burst.
        BurstTagger::new(prev, burst_tee, opt.threshold, "burst".to_string()),
        // Create quad demod raw sample blobs (Vec<Float>) from tagged
        // stream of Floats.
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, 50),
        // A kind of frequency lock.
        Midpointer::new(prev),
        // Symbol sync.
        Wpcr::builder(prev).samp_rate(samp_rate).build(),
        // Turn Vec<Float> into Float.
        VecToStream::new(prev),
        // Turn floats into bits.
        BinarySlicer::new(prev),
        // NRZI decode.
        NrziDecode::new(prev),
        // G3RUH descramble.
        Descrambler::new(prev, 0x21, 0, 16),
        // Decode.
        HdlcDeframer::new(prev, 10, 1500),
    ];

    // Save.
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Run.
    g.run()?;
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-9600-wpcr -- -r ../aprs-9600-50k.c32 -o ../packets"
 * End:
 */
