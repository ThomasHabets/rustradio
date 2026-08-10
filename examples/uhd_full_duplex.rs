//! Exercise simultaneous UHD transmit and receive on one shared USRP.

use anyhow::{Result, ensure};
use clap::Parser;

use rustradio::blocks::{Head, SignalSourceComplex, UhdDevice, UhdSink, UhdSource, VectorSink};
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::{Complex, blockchain, parse_frequency};

#[derive(Debug, Parser)]
#[command(about, version)]
struct Opt {
    /// UHD device address string.
    #[arg(long, default_value = "type=b200")]
    device: String,

    /// Shared RX/TX center frequency in Hz.
    #[arg(long, value_parser = parse_frequency, default_value = "2.45G")]
    frequency: f64,

    /// RX/TX sample rate in samples per second.
    #[arg(long, value_parser = parse_frequency, default_value = "250k")]
    sample_rate: f64,

    /// Baseband test-tone frequency in Hz.
    #[arg(long, value_parser = parse_frequency, default_value = "25k")]
    tone_frequency: f64,

    /// Number of seconds to transmit and capture.
    #[arg(long, default_value_t = 0.5)]
    duration: f64,

    /// Transmit gain in dB.
    #[arg(long, default_value_t = 0.0)]
    tx_gain: f64,

    /// Receive gain in dB.
    #[arg(long, default_value_t = 30.0)]
    rx_gain: f64,

    /// Transmit antenna name.
    #[arg(long, default_value = "TX/RX")]
    tx_antenna: String,

    /// Receive antenna name.
    #[arg(long, default_value = "RX2")]
    rx_antenna: String,

    /// Clock source for motherboard 0, such as `gpsdo`.
    #[arg(long)]
    clock_source: Option<String>,

    /// Time source for motherboard 0, such as `gpsdo`.
    #[arg(long)]
    time_source: Option<String>,

    /// Logging verbosity.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose as usize)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    ensure!(opt.frequency > 0.0 && opt.frequency.is_finite());
    ensure!(
        opt.sample_rate > 0.0 && opt.sample_rate.is_finite() && opt.sample_rate <= f32::MAX as f64
    );
    ensure!(opt.tone_frequency.is_finite());
    ensure!(opt.duration > 0.0 && opt.duration.is_finite());
    let sample_count = opt.duration * opt.sample_rate;
    ensure!(sample_count >= 1.0 && sample_count.is_finite() && sample_count <= usize::MAX as f64);
    let sample_count = sample_count as u64;
    let sample_count_usize = usize::try_from(sample_count)?;

    let device = UhdDevice::open(&opt.device)?;
    if let Some(clock_source) = &opt.clock_source {
        device.configure(|usrp| usrp.set_clock_source(clock_source, 0))?;
    }
    if let Some(time_source) = &opt.time_source {
        device.configure(|usrp| usrp.set_time_source(time_source, 0))?;
    }
    let mut graph = MTGraph::new();
    let rx = blockchain![
        graph,
        rx,
        UhdSource::builder(&device, opt.frequency, opt.sample_rate)
            .gain(opt.rx_gain)
            .antenna(&opt.rx_antenna)
            .build()?,
        Head::new(rx, sample_count),
    ];
    let rx_sink = VectorSink::new(rx, sample_count_usize);
    let rx_hook = rx_sink.hook();
    graph.add(Box::new(rx_sink));

    let tx = blockchain![
        graph,
        tx,
        SignalSourceComplex::new(opt.sample_rate as _, opt.tone_frequency as _, 0.05),
        Head::new(tx, sample_count),
    ];
    graph.add(Box::new(
        UhdSink::builder(&device, opt.frequency, opt.sample_rate)
            .gain(opt.tx_gain)
            .antenna(&opt.tx_antenna)
            .build(tx)?,
    ));
    graph.run()?;

    let received = rx_hook.data();
    ensure!(received.samples().len() == sample_count_usize);
    for key in [
        "UhdSource::frequency",
        "UhdSource::sample_rate",
        "UhdSource::time_ns",
    ] {
        ensure!(
            received.tags().iter().any(|tag| tag.key() == key),
            "missing UHD receive tag {key}"
        );
    }
    for key in [
        "UhdSource::burst",
        "UhdSource::more_fragments",
        "UhdSource::fragment_offset",
        "UhdSource::out_of_sequence",
        "UhdSource::error",
        "UhdSource::error_kind",
        "UhdSource::error_message",
    ] {
        ensure!(
            !received.tags().iter().any(|tag| tag.key() == key),
            "unexpected UHD receive tag {key}"
        );
    }

    let mean_power = received
        .samples()
        .iter()
        .map(Complex::norm_sqr)
        .sum::<f32>()
        / received.samples().len() as f32;
    let metadata_errors = received
        .tags()
        .iter()
        .filter(|tag| tag.key() == "UhdSource::error_kind")
        .count();
    println!(
        "received {} samples, {} tags ({} metadata errors), mean power {:.2} dBFS",
        received.samples().len(),
        received.tags().len(),
        metadata_errors,
        10.0 * mean_power.log10(),
    );
    for tag in received.tags() {
        println!("Tag: {tag:?}");
    }
    Ok(())
}
