//!
//! Super ugly test code for capturing AX.25 frames.
use std::collections::VecDeque;
use std::io::Write;
use std::time::SystemTime;

use anyhow::Result;
use log::{debug, info, warn};
use structopt::StructOpt;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::Streamp;
use rustradio::{Complex, Error};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[cfg(feature = "rtlsdr")]
    #[structopt(long = "freq", default_value = "144800000")]
    freq: u64,

    #[structopt(short = "v", default_value = "0")]
    verbose: usize,

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

fn range_compare(a: &[u8], b: impl Iterator<Item = u8>) -> bool {
    a.iter().zip(b).map(|(a, b)| *a == b).all(|x| x)
}

fn find_pattern(needle: &[u8], haystack: &[u8]) -> Option<usize> {
    for i in 0..(haystack.len() - needle.len()) {
        if range_compare(needle, haystack.iter().skip(i).take(needle.len()).copied()) {
            return Some(i);
        }
    }
    None
}

impl Block for HdlcDeframer {
    fn block_name(&self) -> &'static str {
        "HDLC Deframer"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        // TODO: Problems with this code:
        // * Since start and end marker are the same, you can miss the
        //   real marker if it's mistaken for and end marker.
        // * The history drain logic is all wrong, likely missing
        //   packets.
        // * Likely to trigger false positives because it's currently
        //   only checking for one sync ("flag") byte.
        // * Doesn't check CRC at all.
        // * Latency is way too high, as we collect too many bits
        //   before even looking.
        //
        // This is basically proof of concept at this point.

        //let cac = str2bits("01111110011111100111111001111110");
        let cac = str2bits("01111110");
        let cac1 = str2bits("01111110");
        let min_bytes = 400;
        let max_bytes = 1500;
        {
            let mut input = self.src.lock()?;
            if input.is_empty() {
                return Ok(BlockRet::Noop);
            }
            self.history.extend(input.iter());
            if self.history.len() > max_bytes * 8 {
                self.history
                    .drain(0..(self.history.len() - (max_bytes * 8)));
            }
            input.clear();
        }

        let max_len = cac.len() + min_bytes * 8;
        debug!("looking through {}", self.history.len());
        while self.history.len() > max_len {
            if !range_compare(&cac, self.history.iter().copied().take(cac.len())) {
                self.history.drain(0..1);
                continue;
            }
            while range_compare(&cac1, self.history.iter().copied().skip(8).take(cac.len())) {
                self.history.drain(0..8);
            }
            debug!("Found CAC! {}", self.history.len());

            // Find end marker.
            let bits;
            if let Some(i) = find_pattern(
                &str2bits("01111110"),
                &self
                    .history
                    .iter()
                    .skip(cac1.len())
                    .copied()
                    .collect::<Vec<_>>(),
            ) {
                debug!("Found end marker at {}", i);
                bits = self
                    .history
                    .iter()
                    .skip(8)
                    .take(i)
                    .copied()
                    .collect::<Vec<_>>();
                self.history.drain(0..i); // Drains the first CAC, the content, but not the supposedly ending CAC.
            } else {
                debug!("No end marker");
                return Ok(BlockRet::Ok);
            }

            // Unstuff.
            let mut ones = 0;
            let mut unstuffed = Vec::new();
            for bit in &bits {
                if *bit == 1 {
                    if ones == 5 {
                        debug!("Too many ones {} {:?}, packet bad", ones, bits);
                        return Ok(BlockRet::Ok);
                    }
                    ones += 1;
                    unstuffed.push(1);
                } else {
                    if ones != 5 {
                        unstuffed.push(0);
                    }
                    ones = 0;
                }
            }
            if unstuffed.len() % 8 > 0 {
                warn!(
                    "Packet not multiple of 8 bits: {} % 8 = {}",
                    unstuffed.len(),
                    unstuffed.len() % 8
                );
                return Ok(BlockRet::Ok);
            }
            debug!("Unstuffed len: {} -> {}", bits.len(), unstuffed.len());
            let mut bytes = Vec::new();
            for i in (0..unstuffed.len()).step_by(8) {
                bytes.push(bits2byte(&unstuffed[i..i + 8]));
            }
            let mut f = std::fs::File::create(format!(
                "packets/{}",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_micros()
            ))?;
            f.write_all(&bytes)?;
            info!("Captured packet: {:0>2x?}", bytes);
            return Ok(BlockRet::Ok);
        }
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

    let (prev, samp_rate) = if true {
        #[cfg(feature = "rtlsdr")]
        {
            // Source.
            let samp_rate = 300_000.0;
            let prev = add_block![g, RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?];

            // Decode.
            let prev = add_block![g, RtlSdrDecode::new(prev)];

            // Filter.
            let taps = rustradio::fir::low_pass_complex(samp_rate, 20_000.0, 100.0);
            let prev = add_block![g, FftFilter::new(prev, &taps)];

            // Resample.
            let new_samp_rate = 50_000.0;
            let prev = add_block![
                g,
                RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
            ];
            let samp_rate = new_samp_rate;
            (prev, samp_rate)
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("rtlsdr feature not enabled")
    } else {
        let samp_rate = 50_000.0;
        //let prev = add_block![g, FileSource::<Complex>::new("aprs-50k.c32", false)?];
        let prev = add_block![g, FileSource::<Complex>::new("test-50k.c32", false)?];
        (prev, samp_rate)
    };

    // Save I/Q to file.
    /*
    let (prev, b) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        b,
        "test.c32",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

    // TODO: AGC step?
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];
    let prev = add_block![g, Hilbert::new(prev, 65)];
    let prev = add_block![g, QuadratureDemod::new(prev, 1.0)];

    let taps = rustradio::fir::low_pass(samp_rate, 2400.0, 100.0);
    let prev = add_block![g, FftFilterFloat::new(prev, &taps)];

    let freq1 = 1200.0;
    let freq2 = 2200.0;
    let center_freq = freq1 + (freq2 - freq1) / 2.0;
    let prev = add_block![
        g,
        AddConst::new(prev, -center_freq * 2.0 * std::f32::consts::PI / samp_rate)
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
    let prev = add_block![g, ZeroCrossing::new(prev, samp_rate / baud, 0.1)];
    let prev = add_block![g, BinarySlicer::new(prev)];

    // Delay xor.
    let (a, b) = add_block![g, Tee::new(prev)];
    let delay = add_block![g, Delay::new(a, 1)];
    let prev = add_block![g, Xor::new(delay, b)];

    let prev = add_block![g, XorConst::new(prev, 1u8)];

    // Save bits to file.
    /*
    let (a, prev) = add_block![g, Tee::new(prev)];
    g.add(Box::new(FileSink::new(
        a,
        "test.u8",
        rustradio::file_sink::Mode::Overwrite,
    )?));
     */

    g.add(Box::new(HdlcDeframer::new(prev)));

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
