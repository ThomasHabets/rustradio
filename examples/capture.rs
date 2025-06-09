/*!
Example broadcast FM receiver, sending output to an Au file.
 */
#[cfg(feature = "soapysdr")]
mod internal {
    use anyhow::Result;
    use clap::Parser;
    use log::warn;

    use rustradio::blocks::*;
    use rustradio::file_sink::Mode;
    use rustradio::graph::Graph;
    use rustradio::graph::GraphRunner;
    use rustradio::{Float, blockchain};

    #[derive(clap::Parser, Debug)]
    #[command(version, about)]
    struct Opt {
        #[arg(short)]
        driver: String,

        #[arg(short)]
        output: std::path::PathBuf,

        // Unused if soapysdr feature not enabled.
        #[allow(dead_code)]
        #[arg(long = "freq", default_value = "100000000")]
        freq: u64,

        // Unused if soapysdr feature not enabled.
        #[allow(dead_code)]
        #[arg(long = "gain", default_value = "20")]
        gain: i32,

        #[arg(short, default_value = "0")]
        verbose: usize,

        #[arg(long = "volume", default_value = "1.0")]
        volume: Float,
    }

    pub fn main() -> Result<()> {
        println!("soapy_fm receiver example");
        let opt = Opt::parse();
        stderrlog::new()
            .module(module_path!())
            .module("rustradio")
            .quiet(false)
            .verbosity(opt.verbose)
            .timestamp(stderrlog::Timestamp::Second)
            .init()?;

        let mut g = Graph::new();
        let samp_rate = 1_024_000.0f32;

        let dev = soapysdr::Device::new(&*opt.driver)?;
        let prev = blockchain![
            g,
            prev,
            SoapySdrSource::builder(&dev, opt.freq as f64, samp_rate as f64)
                .igain(opt.gain as f64)
                .build()?
        ];

        // Filter.
        let taps = rustradio::fir::low_pass_complex(
            samp_rate,
            100_000.0,
            1000.0,
            &rustradio::window::WindowType::Hamming,
        );
        let prev = blockchain![g, prev, FftFilter::new(prev, taps)];

        // Resample.
        let new_samp_rate = 200_000.0;
        let prev = blockchain![
            g,
            prev,
            RationalResampler::builder()
                .deci(samp_rate as usize)
                .interp(new_samp_rate as usize)
                .build(prev)?,
        ];
        let samp_rate = new_samp_rate;

        // TODO: Add broadcast FM deemph.

        // Quad demod.
        let prev = blockchain![g, prev, QuadratureDemod::new(prev, 1.0)];

        let taps = rustradio::fir::low_pass(
            samp_rate,
            44_100.0,
            500.0,
            &rustradio::window::WindowType::Hamming,
        );
        //let audio_filter = FirFilter::new(prev, &taps);
        let prev = blockchain![g, prev, FftFilterFloat::new(prev, &taps)];

        // Resample audio.
        let new_samp_rate = 48_000.0;
        let prev = blockchain![
            g,
            prev,
            RationalResampler::builder()
                .deci(samp_rate as usize)
                .interp(new_samp_rate as usize)
                .build(prev)?,
        ];
        let _samp_rate = new_samp_rate;

        // Change volume.
        let prev = blockchain![
            g,
            prev,
            MultiplyConst::new(prev, opt.volume),
            // Convert to .au.
            AuEncode::new(prev, rustradio::au::Encoding::Pcm16, 48000, 1),
        ];

        // Save to file.
        g.add(Box::new(FileSink::new(prev, opt.output, Mode::Overwrite)?));

        let cancel = g.cancel_token();
        ctrlc::set_handler(move || {
            warn!("Got Ctrl-C");
            eprintln!("\n");
            cancel.cancel();
        })
        .expect("failed to set Ctrl-C handler");
        eprintln!("Running loop");
        g.run()?;
        eprintln!("{}", g.generate_stats().unwrap());
        Ok(())
    }
}

#[cfg(feature = "soapysdr")]
fn main() -> anyhow::Result<()> {
    internal::main()
}

#[cfg(not(feature = "soapysdr"))]
fn main() {
    panic!("This example only works with -F soapysdr");
}
