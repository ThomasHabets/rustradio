/*! AX.25 9600bps receiver.

Can be used to receive APRS over the air with RTL-SDR or from
complex I/Q saved to a file.

```no_run
$ mkdir captured
$ ./ax25-9600-rx -r captured.c32 --samp_rate 50000 -o captured
[…]
$ ./ax25-9600-rx --rtlsdr -o captured -v 2
[…]
```
*/
use std::path::PathBuf;

use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::{Complex, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(long = "audio", short = "a")]
    audio: bool,

    #[structopt(long = "out", short = "o")]
    output: PathBuf,

    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "freq", default_value = "144800000")]
    freq: u64,

    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "gain", default_value = "20")]
    gain: i32,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    #[structopt(long = "rtlsdr")]
    rtlsdr: bool,

    #[structopt(long = "sample_rate", default_value = "300000")]
    samp_rate: u32,

    #[structopt(short = "r")]
    read: Option<String>,
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
        .init()?;

    let mut g = Graph::new();

    // TODO: this is a complete mess.
    let (prev, samp_rate) = if opt.audio {
        if let Some(read) = opt.read {
            let prev = add_block![g, FileSource::new(&read, false)?];
            let prev = add_block![g, AuDecode::new(prev)];

            /*
            let (prev, b) = add_block![g, Tee::new(prev)];
            g.add(Box::new(FileSink::new(
                b,
                "debug/00-audio.f32",
                rustradio::file_sink::Mode::Overwrite,
            )?));
             */

            (prev, opt.samp_rate as Float)
        } else {
            panic!("Audio can only be read from file")
        }
    } else {
        let prev = if let Some(read) = opt.read {
            add_block![g, FileSource::<Complex>::new(&read, false)?]
        } else if opt.rtlsdr {
            #[cfg(feature = "rtlsdr")]
            {
                // Source.
                let prev = add_block![g, RtlSdrSource::new(opt.freq, opt.samp_rate, opt.gain)?];

                // Decode.
                let prev = add_block![g, RtlSdrDecode::new(prev)];
                prev
            }
            #[cfg(not(feature = "rtlsdr"))]
            panic!("rtlsdr feature not enabled")
        } else {
            panic!("Need to provide either --rtlsdr or -r")
        };
        let samp_rate = opt.samp_rate as Float;

        /*
                let (prev, b) = add_block![g, Tee::new(prev)];
                g.add(Box::new(FileSink::new(
                    b,
                    "debug/00-unfiltered.c32",
                    rustradio::file_sink::Mode::Overwrite,
                )?));
        */

        // Filter RF.
        let taps = rustradio::fir::low_pass_complex(samp_rate, 12_500.0, 100.0);
        let prev = add_block![g, FftFilter::new(prev, &taps)];
        // Resample RF.
        let new_samp_rate = 50_000.0;
        let prev = add_block![
            g,
            RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
        ];
        let samp_rate = new_samp_rate;

        let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];
        (prev, samp_rate)
    };

    //let taps = rustradio::fir::low_pass(samp_rate, 20_000.0, 100.0);
    //let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    let baud = 9600.0;
    let prev = add_block![g, ZeroCrossing::new(prev, samp_rate / baud, 0.1)];
    let prev = add_block![g, BinarySlicer::new(prev)];

    // Delay xor, aka NRZI decode.
    let prev = add_block![g, NrziDecode::new(prev)];

    // G3RUH descramble.
    let prev = add_block![g, Descrambler::new(prev, 0x21, 0, 16)];

    // Decode.
    let prev = add_block![g, HdlcDeframer::new(prev, 10, 1500)];

    g.add(Box::new(PduWriter::new(prev, opt.output)));

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    // Run.
    eprintln!("Running…");
    let st = std::time::Instant::now();
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-9600-rx -- -r ../aprs-50k.c32 --sample_rate 50000 -o ../packets"
 * End:
 */
