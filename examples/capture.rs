//! SigMF capture
use anyhow::Result;
use clap::Parser;
use log::warn;
use std::io::Write;

use rustradio::block::BlockRet;
use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::parse_frequency;
use rustradio::sigmf::{self, Annotation, SigMF};
use rustradio::stream::{ReadStream, TagValue, WriteStream};
use rustradio::{Complex, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    driver: String,

    #[arg(short)]
    output: std::path::PathBuf,

    #[arg(long = "freq", value_parser=parse_frequency, default_value = "100m")]
    freq: f64,

    #[arg(long, value_parser=parse_frequency, default_value = "300000")]
    samp_rate: f64,

    #[arg(long = "gain", default_value = "0.3")]
    gain: f64,

    #[arg(short, default_value = "0")]
    verbose: usize,

    /// Set time source.
    #[arg(long)]
    time_source: Option<String>,

    /// Set clock source.
    #[arg(long)]
    clock_source: Option<String>,

    /// Enable GPS coordinates.
    #[arg(long)]
    gps_coordinates: bool,
}

#[derive(rustradio_macros::Block)]
#[rustradio(new)]
struct Metadata {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
    sigmf: SigMF,
    tx: std::sync::mpsc::Sender<SigMF>,
    #[rustradio(default)]
    pos: u64,

    #[rustradio(default)]
    frequency: Option<f64>,
}

impl rustradio::block::Block for Metadata {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        loop {
            let (i, tags) = self.src.read_buf()?;
            if i.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let n = std::cmp::min(i.len(), o.len());
            o.slice()[..n].copy_from_slice(&i.slice()[..n]);
            i.consume(n);
            o.produce(n, &tags);
            let mut new_capture = false;
            for tag in tags {
                match (tag.key(), tag.val()) {
                    ("SoapySdrSource::frequency", TagValue::Float(f)) => {
                        let old = self.frequency;
                        self.frequency = Some(*f as _);
                        if old != self.frequency {
                            new_capture = true;
                        }
                    }
                    ("SoapySdrSource::hardware", TagValue::String(hw)) => {
                        if self.sigmf.global.core_hw.is_none() {
                            self.sigmf.global.core_hw = Some(hw.to_string());
                        }
                    }
                    // TODO: Geolocation.
                    _ => {}
                }
                self.sigmf.annotations.push(Annotation {
                    core_sample_start: self.pos + tag.pos() as u64,
                    core_generator: Some("RustRadio".to_string()),
                    core_label: Some(format!("{} {}", tag.key(), tag.val())),
                    ..Default::default()
                });
            }
            if new_capture {
                self.sigmf.captures.push(sigmf::Capture {
                    core_sample_start: self.pos,
                    core_frequency: self.frequency,
                    // TODO: datetime.
                    ..Default::default()
                });
            }
            self.pos += n as u64;
        }
    }
}

impl Drop for Metadata {
    fn drop(&mut self) {
        // TODO: set sha512
        self.tx.send(std::mem::take(&mut self.sigmf)).unwrap()
    }
}

fn main() -> Result<()> {
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

    let dev = soapysdr::Device::new(&*opt.driver)?;
    if let Some(clock) = &opt.clock_source {
        dev.set_clock_source(clock.as_bytes())?;
    }
    if let Some(time) = &opt.time_source {
        dev.set_time_source(time.as_bytes())?;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let prev = blockchain![
        g,
        prev,
        SoapySdrSource::builder(&dev, opt.freq, opt.samp_rate)
            .igain(opt.gain)
            .gps_coordinates(opt.gps_coordinates)
            .build()?,
        Metadata::new(
            prev,
            SigMF {
                global: sigmf::Global {
                    core_version: sigmf::VERSION.to_string(),
                    core_datatype: "cf32".to_string(),
                    core_sample_rate: Some(opt.samp_rate),
                    core_recorder: Some("RustRadio".to_string()),
                    // TODO:
                    // * author
                    // * description
                    // * hw
                    // * license
                    ..Default::default()
                },
                ..Default::default()
            },
            tx
        ),
    ];

    // Save to file.
    let mode = Mode::Overwrite;
    g.add(Box::new(
        FileSink::builder(opt.output.with_extension("sigmf-data"))
            .mode(mode)
            .build(prev)?,
    ));

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
    drop(g);
    {
        let sigmf = rx.recv().unwrap();
        let ser = serde_json::to_string(&sigmf)?;
        let metaname = opt.output.with_extension("sigmf-meta");
        let mut meta = match mode {
            Mode::Create => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&metaname),
            Mode::Overwrite => std::fs::File::create(&metaname),
            Mode::Append => panic!("can't happen"),
        }
        .map_err(|e| rustradio::Error::msg(format!("Failed to create {metaname:?}: {e}")))?;
        meta.write_all(ser.as_bytes())?;
        meta.flush()?;
    }
    Ok(())
}
