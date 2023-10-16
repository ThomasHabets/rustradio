use std::collections::VecDeque;
use std::io::Write;

use anyhow::Result;
//use log::warn;
use structopt::StructOpt;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::Streamp;
use rustradio::{Complex, Error};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    // Unused if rtlsdr feature not enabled.
    #[structopt(long = "freq", default_value = "100000000")]
    freq: u64,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

    // Unused if rtlsdr feature not enabled.
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

struct HdlcDeframer {
    src: Streamp<u8>,
    history: VecDeque<u8>,
}

impl HdlcDeframer {
    fn new(src: Streamp<u8>) -> Self {
        Self {
            src,
            history: VecDeque::new(),
        }
    }
}

impl Block for HdlcDeframer {
    fn block_name(&self) -> &'static str {
        "HDLC Deframer"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let cac = str2bits("01111110011111100111111001111110");
        {
            let mut input = self.src.lock()?;
            if input.is_empty() {
                return Ok(BlockRet::Noop);
            }
            self.history.extend(input.iter());
            input.clear();
        }

        let max_bytes = 400;
        let max_len = cac.len() + max_bytes;
        let size = self.history.len();
        if size < max_len {
            return Ok(BlockRet::Ok);
        }

        for i in 0..(size - max_len) {
            let equal = cac
                .iter()
                .zip(self.history.range(i..(i + cac.len())))
                .map(|(a, b)| a == b)
                .all(|x| x);
            if !equal {
                continue;
            }
            println!("Found packet!");

            let start = i + cac.len();
            let mut bytes = Vec::new();
            for j in (start..(start + max_len)).step_by(8) {
                let t = self.history.iter().skip(j).take(8);
                bytes.push(bits2byte(&t.copied().collect::<Vec<_>>()));
            }
            let mut r = 0;
            while bytes[r] == 0x7e {
                r += 1;
            }
            let bytes = &bytes[r..];
            let mut fin = Vec::new();
            for b in bytes {
                if *b == 0x7e {
                    break;
                }
                fin.push(*b);
            }
            //if bytes[0] != 0x7f {
            if bytes[0] != 0xfe {
                let mut f = std::fs::File::create(format!("packets/p"))?;
                f.write_all(&fin)?;
                println!("{:.0x?}", fin);
            }
        }
        self.history.drain(0..(size - max_len));
        Ok(BlockRet::Ok)
    }
}

fn bits2byte(data: &[u8]) -> u8 {
    assert!(data.len() == 8);
    data[7] << 7
        | data[6] << 6
        | data[5] << 5
        | data[4] << 4
        | data[3] << 3
        | data[2] << 2
        | data[1] << 1
        | data[0]
}

fn str2bits(s: &str) -> Vec<u8> {
    s.chars()
        .map(|ch| match ch {
            '1' => 1,
            '0' => 0,
            _ => panic!("invalid bitstring: {}", s),
        })
        .collect::<Vec<_>>()
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

    let (prev, samp_rate) = if false {
        let samp_rate = 300_000.0;
        let prev = add_block![g, RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?];
        let prev = add_block![g, RtlSdrDecode::new(prev)];
        (prev, samp_rate)
    } else {
        let samp_rate = 50_000.0;
        let prev = add_block![g, FileSource::<Complex>::new("aprs-50k.c32", false)?];
        (prev, samp_rate)
    };

    // TODO: AGC step?
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];
    let prev = add_block![g, Hilbert::new(prev, 65)];
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    let taps = rustradio::fir::low_pass(samp_rate, 2400.0, 100.0);
    let prev = add_block![g, FftFilterFloat::new(prev, &taps)];
    let prev = add_block![g, AddConst::new(prev, -(0.15 + (0.28 - 0.15) / 2.0))];

    let baud = 1200.0;
    let prev = add_block![g, ZeroCrossing::new(prev, samp_rate / baud, 0.1)];
    let prev = add_block![g, BinarySlicer::new(prev)];

    // Delay xor.
    let (a, b) = add_block![g, Tee::new(prev)];
    let delay = add_block![g, Delay::new(a, 1)];
    let prev = add_block![g, Xor::new(delay, b)];

    let prev = add_block![g, XorConst::new(prev, 1u8)];

    let (a, b) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "test.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));
    g.add(Box::new(HdlcDeframer::new(b)));

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
