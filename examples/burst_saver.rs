/*! Burst saver.

Listen for power bursts, and save them as separate files in an output
directory.
*/
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use log::info;
use structopt::StructOpt;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::{new_streamp, Streamp, Tag, TagPos, TagValue};
use rustradio::{Complex, Error, Float, Sample};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(long = "out", short = "o")]
    output: PathBuf,

    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "freq", default_value = "144800000")]
    freq: u64,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    #[structopt(long = "rtlsdr")]
    rtlsdr: bool,

    #[structopt(long = "samp_rate", default_value = "300000")]
    samp_rate: u32,

    #[structopt(short = "r")]
    read: Option<String>,

    #[structopt(long = "threshold", default_value = "0.0001")]
    threshold: Float,

    #[structopt(long = "iir_alpha", default_value = "0.01")]
    iir_alpha: Float,

    #[structopt(long = "delay", default_value = "3000")]
    delay: usize,

    #[structopt(long = "tail", default_value = "5000")]
    tail: usize,

    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "gain", default_value = "20")]
    gain: i32,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
}

struct StreamToPdu<T> {
    src: Streamp<T>,
    dst: Streamp<Vec<T>>,
    tag: String,
    buf: Vec<T>,
    endcounter: Option<usize>,
    max_size: usize,
    tail: usize,
}

impl<T> StreamToPdu<T> {
    fn new(src: Streamp<T>, tag: String, max_size: usize, tail: usize) -> Self {
        Self {
            src,
            tag,
            dst: new_streamp(),
            buf: Vec::new(),
            endcounter: None,
            max_size,
            tail,
        }
    }
    fn out(&self) -> Streamp<Vec<T>> {
        self.dst.clone()
    }
}

fn get_tag_val_bool(tags: &HashMap<(TagPos, String), Tag>, pos: TagPos, key: &str) -> Option<bool> {
    if let Some(tag) = tags.get(&(pos, key.to_string())) {
        match tag.val() {
            TagValue::Bool(b) => Some(*b),
            _ => None,
        }
    } else {
        None
    }
}

impl<T> Block for StreamToPdu<T>
where
    T: Copy + Sample,
{
    fn block_name(&self) -> &'static str {
        "StreamToPdu"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        if input.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        // TODO: we actually only care about one single tag,
        // and I think we should drop the rest no matter what.
        let tags = input
            .tags()
            .into_iter()
            .map(|t| ((t.pos(), t.key().to_string()), t))
            .collect::<HashMap<(TagPos, String), Tag>>();
        for (i, sample) in input.iter().enumerate() {
            if let Some(0) = self.endcounter {
                let mut delme = Vec::new();
                std::mem::swap(&mut delme, &mut self.buf);
                info!(
                    "Wrote burst of size {} samples, {} bytes",
                    delme.len(),
                    delme.len() * T::size()
                );
                self.dst.lock()?.push(delme);
                self.endcounter = None;
            }
            if let Some(c) = self.endcounter {
                self.buf.push(*sample);
                self.endcounter = Some(c - 1);
            } else {
                if let Some(tv) = get_tag_val_bool(&tags, i as TagPos, &self.tag) {
                    if !tv {
                        self.endcounter = Some(self.tail);
                    } else if !self.buf.is_empty() {
                        self.buf.push(*sample);
                    }
                } else if !self.buf.is_empty() {
                    self.buf.push(*sample);
                }
            }
            if self.buf.len() > self.max_size {
                self.buf.clear();
                self.endcounter = None;
            }
        }
        input.clear();
        Ok(BlockRet::Ok)
    }
}

struct BurstTagger<T> {
    src: Streamp<T>,
    threshold: Float,
    trigger: Streamp<Float>,
    dst: Streamp<T>,
    tag: String,
    last: bool,
}

impl<T> BurstTagger<T> {
    fn new(src: Streamp<T>, trigger: Streamp<Float>, threshold: Float, tag: String) -> Self {
        Self {
            src,
            trigger,
            threshold,
            tag,
            dst: new_streamp(),
            last: false,
        }
    }
    fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T> Block for BurstTagger<T>
where
    T: Copy,
{
    fn block_name(&self) -> &'static str {
        "Burst Tagger"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        let mut trigger = self.trigger.lock()?;
        let n = std::cmp::min(input.available(), trigger.available());
        if n == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut v = Vec::with_capacity(input.available());
        let mut tags = Vec::new();
        for (i, (s, tv)) in input.iter().zip(trigger.iter()).enumerate().take(n) {
            let cur = *tv > self.threshold;
            if cur != self.last {
                tags.push(Tag::new(
                    i as u64,
                    self.tag.clone(),
                    if cur {
                        TagValue::Bool(true)
                    } else {
                        TagValue::Bool(false)
                    },
                ));
            }
            self.last = cur;
            v.push(*s);
        }
        self.dst.lock()?.write_tags(v.iter().copied(), &tags);
        input.consume(n);
        trigger.consume(n);
        Ok(BlockRet::Ok)
    }
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

    let (prev, samp_rate) = if let Some(read) = opt.read {
        let prev = add_block![g, FileSource::<Complex>::new(&read, false)?];
        (prev, opt.samp_rate as Float)
    } else if opt.rtlsdr {
        #[cfg(feature = "rtlsdr")]
        {
            // Source.
            let prev = add_block![g, RtlSdrSource::new(opt.freq, opt.samp_rate, opt.gain)?];

            // Decode.
            let prev = add_block![g, RtlSdrDecode::new(prev)];
            (prev, opt.samp_rate as Float)
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("rtlsdr feature not enabled")
    } else {
        panic!("Need to provide either --rtlsdr or -r")
    };

    // Filter RF.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 20_000.0, 100.0);
    let prev = add_block![g, FftFilter::new(prev, &taps)];

    // Resample RF.
    let new_samp_rate = 50_000.0;
    let prev = add_block![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    let (datapath, magpath) = add_block![g, Tee::new(prev)];
    let magpath = add_block![g, ComplexToMag2::new(magpath)];
    let magpath = add_block![
        g,
        SinglePoleIIRFilter::new(magpath, opt.iir_alpha).ok_or(Error::new("bad IIR parameters"))?
    ];
    let datapath = add_block![g, Delay::new(datapath, opt.delay)];
    let prev = add_block![
        g,
        BurstTagger::new(datapath, magpath, opt.threshold, "burst".to_string())
    ];

    let prev = add_block![
        g,
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, opt.tail)
    ];
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Set up Ctrl-C.
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
