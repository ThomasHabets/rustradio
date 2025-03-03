/*!
Example broadcast FM receiver, sending output to an Au file.
 */
#[cfg(feature = "soapysdr")]
mod internal {
    use anyhow::Result;
    use clap::Parser;
    use log::warn;

    use rustradio::Float;
    use rustradio::blocks::*;
    use rustradio::file_sink::Mode;
    use rustradio::graph::Graph;
    use rustradio::graph::GraphRunner;

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

    macro_rules! blehbleh {
        ($g:ident, $cons:expr) => {{
            let (block, prev) = $cons;
            $g.add(Box::new(block));
            prev
        }};
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

        let prev = blehbleh![
            g,
            SoapySdrSourceBuilder::new(opt.driver.clone(), opt.freq as f64, samp_rate as f64)
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
        //let audio_filter = FirFilter::new(prev, &taps);
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
            AuEncode::new(prev, rustradio::au::Encoding::Pcm16, 48000, 1)
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
