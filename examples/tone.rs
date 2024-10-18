/*!
Example tone generator.
 */
use anyhow::Result;
use log::warn;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::Float;

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    /// Frequency of tone.
    #[allow(dead_code)]
    #[structopt(long = "freq", default_value = "8000")]
    freq: Float,

    /// Verbosity of debug messages.
    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    /// Tone volume.
    #[structopt(long = "volume", default_value = "0.1")]
    volume: Float,

    /// Audio output rate.
    #[structopt(default_value = "48000")]
    audio_rate: u32,
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
    println!("tone generator receiver example");
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let mut g = Graph::new();

    // Two ways of getting a real sine wave, just as examples.
    let prev = if false {
        add_block![
            g,
            SignalSourceFloat::new(opt.audio_rate as Float, opt.freq, opt.volume)
        ]
    } else {
        let prev = add_block![
            g,
            SignalSourceComplex::new(opt.audio_rate as Float, opt.freq, opt.volume)
        ];
        add_block![
            g,
            MapBuilder::new(prev, |x| x.re)
                .name("ComplexToReal".to_owned())
                .build()
        ]
    };

    g.add(Box::new(AudioSink::new(prev, opt.audio_rate as u64)?));

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
