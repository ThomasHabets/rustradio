/*! Test program for whole packet clock recovery.

This is the same as ax25-1200-rx.rs, except it has fewer options
(e.g. only supports reading from a file), and uses WPCR instead of
ZeroCrossing symbol sync.
*/
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use anyhow::Result;
use structopt::StructOpt;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::{new_streamp, Streamp};
use rustradio::{Error, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(short = "r")]
    read: String,

    #[structopt(long = "sample_rate", default_value = "50000")]
    sample_rate: Float,

    #[structopt(short = "o")]
    output: PathBuf,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,
}

pub struct VecToStream<T> {
    src: Streamp<Vec<T>>,
    dst: Streamp<T>,
}

impl<T> VecToStream<T> {
    pub fn new(src: Streamp<Vec<T>>) -> Self {
        Self {
            src,
            dst: new_streamp(),
        }
    }
    pub fn out(&self) -> Streamp<T> {
        self.dst.clone()
    }
}

impl<T: Copy> Block for VecToStream<T> {
    fn block_name(&self) -> &'static str {
        "VecToStream"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut i = self.src.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.lock()?;
        for v in i.iter() {
            o.write_slice(v);
        }
        i.clear();
        Ok(BlockRet::Ok)
    }
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
}

fn load(filename: &str) -> Result<Vec<f32>> {
    let mut file = File::open(filename)?;

    let mut v = Vec::new();
    let mut buffer = [0u8; 4];
    while let Ok(bytes_read) = file.read(&mut buffer) {
        if bytes_read == 0 {
            break;
        }
        let float_value = f32::from_le_bytes(buffer);
        v.push(float_value);
    }
    Ok(v)
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()
        .unwrap();

    let samples = load(&opt.read)?;

    let src = rustradio::stream::new_streamp();
    src.lock().unwrap().push(samples);

    let mut g = Graph::new();

    // TODO: read I/Q from file, quad demod, etc.

    // Symbol sync.
    let prev = add_block![g, WpcrBuilder::new(src).samp_rate(opt.sample_rate).build()];
    let prev = add_block![g, VecToStream::new(prev)];

    // Delay xor.
    let (a, b) = add_block![g, Tee::new(prev)];
    let delay = add_block![g, Delay::new(a, 1)];
    let prev = add_block![g, Xor::new(delay, b)];
    let prev = add_block![g, XorConst::new(prev, 1u8)];

    // Decode.
    let prev = add_block![g, HdlcDeframer::new(prev, 10, 1500)];

    // Save.
    g.add(Box::new(PduWriter::new(prev, opt.output)));

    // Run.
    let st = std::time::Instant::now();
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
