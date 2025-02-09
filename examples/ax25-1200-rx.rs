/*! AX.25 1200bps Bell 202 receiver.

Can be used to receive APRS over the air with RTL-SDR or from
complex I/Q saved to a file.

```no_run
$ mkdir captured
$ ./ax25-1200-rx -r captured.c32 --samp_rate 50000 -o captured
[…]
$ ./ax25-1200-rx --rtlsdr -o captured -v 2
[…]
```

Test recordings for this code are at
<http://wa8lmf.net/TNCtest/index.htm>. Note that track 2 should not
be used, as it's incorrectly de-emphasized.

As of 2023-12-27:

* 1031 Dire Wolf, single bit fix up. -P E+ -F 1
* 1015 Dire Wolf, error-free frames only. -P E+
*  906 This code, single bit fix up.
*  906 This code, error-free frames only.
* (from direwolf doc) 70% Kantronics KPC-3 Plus
* (from direwolf doc) 67% Kenwood TM-D710A

## Other useful links.

* <https://github.com/wb2osz/direwolf/raw/master/doc/A-Better-APRS-Packet-Demodulator-Part-1-1200-baud.pdf>
* <https://github.com/wb2osz/direwolf/raw/master/doc/WA8LMF-TNC-Test-CD-Results.pdf>
* <https://github.com/wb2osz/direwolf/blob/master/doc/A-Closer-Look-at-the-WA8LMF-TNC-Test-CD.pdf>
* <https://www.febo.com/packet/layer-one/transmit.html>
* <https://www.febo.com/packet/layer-one/receive.html>

*/
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::stream::ReadStream;
use rustradio::window::WindowType;
use rustradio::Error;
use rustradio::{graph::Graph, mtgraph::MTGraph};
use rustradio::{Complex, Float};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(long, short, help = "Input is an .au file, not I/Q")]
    audio: bool,

    #[arg(long = "out", short, help = "Directory to write packets to")]
    output: Option<PathBuf>,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "freq", default_value = "144800000")]
    freq: u64,

    #[cfg(feature = "rtlsdr")]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,

    #[arg(short = 'v', default_value = "0")]
    verbose: usize,

    #[arg(long)]
    fix_bits: bool,

    #[arg(long = "rtlsdr", help = "Stream I/Q from an RTLSDR")]
    rtlsdr: bool,

    #[arg(long = "clock-file", help = "File to write clock sync data to")]
    clock_file: Option<PathBuf>,

    #[arg(long = "sample_rate")]
    samp_rate: Option<u32>,

    #[arg(short = 'r', help = "Read I/Q from file")]
    read: Option<String>,

    #[arg(long = "fast_fm", help = "Use FastFM for the FM carrier demod")]
    fast_fm: bool,

    #[arg(
        long = "symbol_taps",
        default_value = "0.5,0.5",
        use_value_delimiter = true
    )]
    symbol_taps: Vec<Float>,

    #[arg(long, default_value = "0.5")]
    symbol_max_deviation: Float,

    /// Run multithreaded.
    #[arg(long)]
    multithread: bool,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let (block, prev) = $cons;
        let block = Box::new(block);
        $g.add(block);
        prev
    }};
}

fn get_complex_input(
    g: &mut Box<dyn GraphRunner>,
    opt: &Opt,
) -> Result<(ReadStream<Complex>, f32)> {
    if let Some(ref read) = opt.read {
        let mut b = SigMFSourceBuilder::new(read.clone());
        if let Some(s) = opt.samp_rate {
            b = b.sample_rate(s as f64);
        }
        let (b, prev) = b.build()?;
        let samp_rate = b
            .sample_rate()
            .ok_or(Error::new("SigMF file does not specify sample rate"))?;
        g.add(Box::new(b));
        return Ok((prev, samp_rate as f32));
    }

    if opt.rtlsdr {
        #[cfg(feature = "rtlsdr")]
        {
            let samp = opt
                .samp_rate
                .ok_or(Error::new("Sample rate must be provided for RTLSDR input"))?;
            let prev = add_block![g, RtlSdrSource::new(opt.freq, samp, opt.gain)?];

            // Decode.
            let prev = add_block![g, RtlSdrDecode::new(prev)];
            return Ok((prev, samp as f32));
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("rtlsdr feature not enabled");
    }
    panic!("not read, not rtlsdr");
}

fn get_input(g: &mut Box<dyn GraphRunner>, opt: &Opt) -> Result<(ReadStream<Float>, f32)> {
    if opt.audio {
        if let Some(ref read) = &opt.read {
            let prev = add_block![g, FileSource::new(read, false)?];
            let prev = add_block![
                g,
                AuDecode::new(
                    prev,
                    opt.samp_rate.expect("audio source requires --sample_rate")
                )
            ];
            // TODO: AuDecode should be providing the bitrate.
            return Ok((
                prev,
                opt.samp_rate.ok_or(Error::new(
                    "audio input requires providing a sample rate, for now",
                ))? as f32,
            ));
        }
        panic!("Audio can only be read from file");
    }

    let (prev, samp_rate) = get_complex_input(g, opt)?;
    let taps = rustradio::fir::low_pass_complex(
        samp_rate,
        20_000.0,
        100.0,
        &rustradio::window::WindowType::Hamming,
    );
    #[cfg(feature = "fftw")]
    let e = rustradio::fft_filter::rr_fftw::FftwEngine::new(&taps);
    #[cfg(not(feature = "fftw"))]
    let e = rustradio::fft_filter::rr_rustfft::RustFftEngine::new(&taps);
    let prev = add_block![g, FftFilter::new(prev, e)];
    let new_samp_rate = 50_000.0;
    let prev = add_block![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;
    let prev = if opt.fast_fm {
        // This is faster, but slightly worse.
        add_block![g, FastFM::new(prev)]
    } else {
        add_block![g, QuadratureDemod::new(prev, 1.0)]
    };
    Ok((prev, samp_rate))
}

#[cfg(feature = "fftw")]
#[inline(never)]
fn fft_bench_fftw(
    plan: &mut impl fftw::plan::C2CPlan<Complex = Complex>,
    a: &mut [Complex],
    b: &mut [Complex],
) {
    let _ = plan.c2c(a, b);
}

#[inline(never)]
fn fft_bench_rustfft(fft: &std::sync::Arc<dyn rustfft::Fft<Float>>, a: &mut [Complex]) {
    fft.process(a);
}

#[inline(never)]
fn fft_bench() {
    let n = 1024;
    let iter = 100000;
    // FFTW.
    #[cfg(feature = "fftw")]
    {
        use fftw::array::AlignedVec;
        use fftw::plan::*;
        use fftw::types::*;
        use std::f32::consts::PI;
        let mut plan: C2CPlan32 = C2CPlan::aligned(&[n], Sign::Forward, Flag::MEASURE).unwrap();
        let mut a = AlignedVec::new(n);
        let mut b = AlignedVec::new(n);
        let k0 = 2.0 * PI / n as f32;
        for i in 0..n {
            a[i] = c32::new((k0 * i as f32).cos(), 0.0);
        }
        let start = std::time::Instant::now();
        for _ in 0..iter {
            //plan.c2c(&mut a, &mut b).unwrap();
            fft_bench_fftw(&mut plan, &mut a, &mut b);
        }
        println!("FFTW: {:?}", start.elapsed());
    }

    // rustfft
    {
        let mut planner = rustfft::FftPlanner::new();
        let fft = planner.plan_fft_forward(n);
        let mut v = vec![Complex::default(); n];
        let start = std::time::Instant::now();
        for _ in 0..iter {
            fft_bench_rustfft(&fft, &mut v);
        }
        println!("rustfft: {:?}", start.elapsed());
    }
    panic!();
}

fn main() -> Result<()> {
    if false {
        fft_bench();
    }
    {
        let features = rustradio::check_environment()?;
        print!("{}", rustradio::environment_str(&features));
    }

    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g: Box<dyn GraphRunner> = if opt.multithread {
        Box::new(MTGraph::new())
    } else {
        Box::new(Graph::new())
    };

    let (prev, samp_rate) = get_input(&mut g, &opt)?;
    let prev = add_block![g, Hilbert::new(prev, 65, &WindowType::Hamming)];

    // Can't use FastFM here, because it doesn't work well with
    // preemph'd input.
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    let taps = rustradio::fir::low_pass(
        samp_rate,
        1100.0,
        100.0,
        &rustradio::window::WindowType::Hamming,
    );
    #[cfg(feature = "fftw")]
    let prev = add_block![
        g,
        FftFilterFloat::new(
            prev,
            rustradio::fft_filter::rr_fftw::FftwEngine::new(
                &taps
                    .iter()
                    .copied()
                    .map(|f| Complex::new(f, 0.0))
                    .collect::<Vec<_>>(),
            )
        )
    ];
    #[cfg(not(feature = "fftw"))]
    let prev = add_block![
        g,
        FftFilterFloat::new(
            prev,
            rustradio::fft_filter::rr_rustfft::RustFftEngine::new(
                &taps
                    .iter()
                    .copied()
                    .map(|f| Complex::new(f, 0.0))
                    .collect::<Vec<_>>(),
            )
        )
    ];

    let freq1 = 1200.0;
    let freq2 = 2200.0;
    let center_freq = freq1 + (freq2 - freq1) / 2.0;
    let prev = add_block![
        g,
        add_const(prev, -center_freq * 2.0 * std::f32::consts::PI / samp_rate)
    ];

    /*
    // Save floats to file.
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "test.f32",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */
    let baud = 1200.0;
    let (prev, mut block) = {
        //let block = ZeroCrossing::new(prev, samp_rate / baud, opt.symbol_max_deviation);
        let clock_filter = rustradio::iir_filter::IIRFilter::new(&opt.symbol_taps);
        let (block, out) = SymbolSync::new(
            prev,
            samp_rate / baud,
            opt.symbol_max_deviation,
            Box::new(rustradio::symbol_sync::TEDZeroCrossing::new()),
            Box::new(clock_filter),
        );
        (out, block)
    };

    // Optional clock output.
    let prev = if let Some(clockfile) = opt.clock_file {
        let clock = block.out_clock();
        let (block, a, prev) = Tee::new(prev);
        g.add(Box::new(block));
        let clock = add_block![g, AddConst::new(clock.expect("TODO"), -samp_rate / baud)];
        let clock = add_block![g, ToText::new(vec![a, clock])];
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

    let prev = add_block![g, BinarySlicer::new(prev)];

    // Delay xor, aka NRZI decode.
    let prev = add_block![g, NrziDecode::new(prev)];

    // Save bits to file.
    /*
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "test.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

    let (mut hdlc, prev) = HdlcDeframer::new(prev, 10, 1500);
    hdlc.set_fix_bits(opt.fix_bits);
    g.add(Box::new(hdlc));
    if let Some(o) = opt.output {
        g.add(Box::new(PduWriter::new(prev, o)));
    } else {
        g.add(Box::new(DebugSinkNoCopy::new(prev)));
    }

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        cancel.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    // Run.
    eprintln!("Running…");
    let st = std::time::Instant::now();
    // use gperftools::profiler::PROFILER;
    // PROFILER.lock().unwrap().start("./my-prof.prof").unwrap();
    g.run()?;
    // PROFILER.lock().unwrap().stop().unwrap();
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
/* ---- Emacs variables ----
 * Local variables:
 * compile-command: "cargo run --example ax25-1200-rx -- -r ../aprs-50k.c32 --sample_rate 50000 -o ../packets"
 * End:
 */
