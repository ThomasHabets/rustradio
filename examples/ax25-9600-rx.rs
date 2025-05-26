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

* <https://www.amsat.org/amsat/articles/kd2bd/9k6modem/>
*/
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::{Complex, Float, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(long = "audio", short)]
    audio: bool,

    #[arg(long = "out", short)]
    output: PathBuf,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "freq", default_value = "144800000")]
    freq: u64,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,

    #[arg(short, default_value = "0")]
    verbose: usize,

    #[arg(long = "rtlsdr")]
    rtlsdr: bool,

    #[arg(long = "clock-file", help = "File to write clock sync data to")]
    clock_file: Option<PathBuf>,

    #[arg(long = "sample_rate", default_value = "300000")]
    samp_rate: u32,

    #[arg(short)]
    read: Option<String>,

    #[arg(
        long = "symbol_taps",
        default_value = "0.0001,0.99999999",
        use_value_delimiter = true
    )]
    symbol_taps: Vec<Float>,

    #[arg(long, default_value = "0.1")]
    symbol_max_deviation: Float,
}

fn main() -> Result<()> {
    let opt = Opt::parse();
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
            let prev = blockchain![
                g,
                prev,
                FileSource::new(&read)?,
                AuDecode::new(prev, opt.samp_rate),
            ];

            /*
            let (t, prev, b) = Tee::new(prev);
            g.add(Box::new(t));
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
            blockchain![g, prev, FileSource::<Complex>::new(&read)?]
        } else if opt.rtlsdr {
            #[cfg(feature = "rtlsdr")]
            {
                // Source.
                blockchain![
                    g,
                    prev,
                    RtlSdrSource::new(opt.freq, opt.samp_rate, opt.gain)?,
                    RtlSdrDecode::new(prev),
                ]
            }
            #[cfg(not(feature = "rtlsdr"))]
            panic!("rtlsdr feature not enabled")
        } else {
            panic!("Need to provide either --rtlsdr or -r")
        };
        let samp_rate = opt.samp_rate as Float;

        /*
                let (t, prev, b) = Tee::new(prev);
                g.add(Box::new(t));
                g.add(Box::new(FileSink::new(
                    b,
                    "debug/00-unfiltered.c32",
                    rustradio::file_sink::Mode::Overwrite,
                )?));
        */

        // Filter RF.
        let taps = rustradio::fir::low_pass_complex(
            samp_rate,
            12_500.0,
            100.0,
            &rustradio::window::WindowType::Hamming,
        );
        let new_samp_rate = 50_000.0;
        let prev = blockchain![
            g,
            prev,
            FftFilter::new(prev, taps),
            RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?,
            QuadratureDemod::new(prev, 1.0),
        ];
        (prev, new_samp_rate)
    };

    //let taps = rustradio::fir::low_pass(samp_rate, 20_000.0, 100.0);
    //let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    let baud = 9600.0;
    let (prev, mut block) = {
        let clock_filter = rustradio::iir_filter::IirFilter::new(&opt.symbol_taps);
        let (block, prev) = SymbolSync::new(
            prev,
            samp_rate / baud,
            opt.symbol_max_deviation,
            Box::new(rustradio::symbol_sync::TedZeroCrossing::new()),
            Box::new(clock_filter),
        );
        (prev, block)
    };

    // Optional clock output.
    let prev = if let Some(clockfile) = opt.clock_file {
        let clock = block.out_clock().unwrap();
        let (b, a, prev) = Tee::new(prev);
        g.add(Box::new(b));
        let clock = blockchain![
            g,
            clock,
            AddConst::new(clock, -samp_rate / baud),
            ToText::new(vec![a, clock]),
        ];
        g.add(Box::new(FileSink::new(
            clock,
            clockfile,
            rustradio::file_sink::Mode::Overwrite,
        )?));
        prev
    } else {
        prev
    };
    g.add(Box::new(block));

    let prev = blockchain![
        g,
        prev,
        BinarySlicer::new(prev),
        // Delay xor, aka NRZI decode.
        NrziDecode::new(prev),
        // G3RUH descramble.
        Descrambler::new(prev, 0x21, 0, 16),
        // Decode.
        HdlcDeframer::new(prev, 10, 1500),
    ];

    g.add(Box::new(PduWriter::new(prev, opt.output)));

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    // Run.
    eprintln!("Running…");
    g.run()?;
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-9600-rx -- -r data/aprs-9600-50k.c32 --sample_rate 50000 -o tmp/"
 * End:
 */
