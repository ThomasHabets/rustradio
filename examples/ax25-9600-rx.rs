/*! Test program for decoding G3RUH 9600bps AX.25 using whole packet
clock recovery.
*/
use std::path::PathBuf;

use anyhow::Result;
use structopt::StructOpt;

use rustradio::blocks::*;
use rustradio::graph::Graph;
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

    #[structopt(long = "threshold", default_value = "0.0001")]
    threshold: Float,

    #[structopt(long = "iir_alpha", default_value = "0.01")]
    iir_alpha: Float,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let block = Box::new($cons);
        let prev = block.out();
        $g.add(block);
        prev
    }};
}

// TODO: put these blocks in with the rest of them.

use rustradio::block::{Block, BlockRet};
use rustradio::stream::{new_streamp, Streamp};

/// Descrambler uses an LFSR to descramble bits.
pub struct Descrambler {
    src: Streamp<u8>,
    dst: Streamp<u8>,
    lfsr: Lfsr,
}
impl Descrambler {
    /// Create new descrambler.
    pub fn new(src: Streamp<u8>, mask: u64, seed: u64, len: u8) -> Self {
        Self {
            src,
            dst: new_streamp(),
            lfsr: Lfsr::new(mask, seed, len),
        }
    }
    /// Get output stream.
    pub fn out(&self) -> Streamp<u8> {
        self.dst.clone()
    }
}
struct Lfsr {
    mask: u64,
    len: u8,
    shift_reg: u64,
}

impl Lfsr {
    fn new(mask: u64, seed: u64, len: u8) -> Self {
        assert!(len < 64);
        Self {
            mask,
            len,
            shift_reg: seed,
        }
    }
    fn next(&mut self, i: u8) -> u8 {
        assert!(i <= 1);
        let ret = 1 & (self.shift_reg & self.mask).count_ones() as u8 ^ i;
        self.shift_reg = (self.shift_reg >> 1) | ((i as u64) << self.len);
        ret
    }
}

impl Block for Descrambler {
    fn block_name(&self) -> &'static str {
        "Descrambler"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut i = self.src.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut v = Vec::with_capacity(i.available());
        for bit in i.iter() {
            v.push(self.lfsr.next(*bit));
        }
        i.clear();
        let mut o = self.dst.lock()?;
        o.write_slice(&v);
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
        .init()
        .unwrap();

    let samp_rate = opt.sample_rate;
    let mut g = Graph::new();

    // Read file.
    let prev = add_block![g, FileSource::new(&opt.read, false)?];

    // Filter.
    let taps = rustradio::fir::low_pass_complex(samp_rate, 20_000.0, 100.0);
    let prev = add_block![g, FftFilter::new(prev, &taps)];

    // Resample RF.
    let new_samp_rate = 50_000.0;
    let prev = add_block![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    // Tee out signal strength.
    let (prev, burst_tee) = add_block![g, Tee::new(prev)];
    let burst_tee = add_block![g, ComplexToMag2::new(burst_tee)];
    let burst_tee = add_block![
        g,
        SinglePoleIIRFilter::new(burst_tee, opt.iir_alpha)
            .ok_or(Error::new("bad IIR parameters"))?
    ];

    // Demod.
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    //let (a, prev) = add_block![g, Tee::new(prev)];
    //g.add(Box::new(FileSink::new(a, "audio.u8", rustradio::file_sink::Mode::Overwrite)?));

    // Filter.
    let taps = rustradio::fir::low_pass(samp_rate, 16000.0, 100.0);
    let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    // Tag.
    let prev = add_block![
        g,
        BurstTagger::new(prev, burst_tee, opt.threshold, "burst".to_string())
    ];

    let prev = add_block![
        g,
        StreamToPdu::new(prev, "burst".to_string(), samp_rate as usize, 50)
    ];

    // Symbol sync.
    //let prev = add_block![g, Midpointer::new(prev)];
    let prev = add_block![g, WpcrBuilder::new(prev).samp_rate(opt.sample_rate).build()];

    let prev = add_block![g, VecToStream::new(prev)];

    let prev = add_block![g, AddConst::new(prev, -0.07)];

    /*
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "preslice.f32",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

    let prev = add_block![g, BinarySlicer::new(prev)];

    /*
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "sliced.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

    // Delay xor.
    let (a, b) = add_block![g, Tee::new(prev)];
    let delay = add_block![g, Delay::new(a, 1)];
    let prev = add_block![g, Xor::new(delay, b)];
    let prev = add_block![g, XorConst::new(prev, 1u8)];

    /*
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "after-nrzi.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

    // G3RUH descramble.
    let prev = add_block![g, Descrambler::new(prev, 0x21, 0, 16)];

    /*
    // Save burst stream
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "test.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

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
