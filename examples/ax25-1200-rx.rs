//!
//! Super ugly test code for capturing AX.25 frames.
use std::io::Write;
use std::time::SystemTime;

use anyhow::Result;
use log::{debug, info};
use structopt::StructOpt;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::{new_streamp, Streamp};
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

struct PduWriter {
    src: Streamp<Vec<u8>>,
    dir: String,
}

impl PduWriter {
    /// Create new PduWriter.
    pub fn new(src: Streamp<Vec<u8>>, dir: String) -> Self {
        Self { src, dir }
    }
}

impl Block for PduWriter {
    fn block_name(&self) -> &'static str {
        "PduWriter"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        for packet in input.iter() {
            //let bytes = Vec::new();
            let mut f = std::fs::File::create(format!(
                "{}/{}",
                self.dir,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_micros()
            ))
            .unwrap();
            f.write_all(packet).unwrap();
        }
        input.clear();
        Ok(BlockRet::Ok)
    }
}

enum State {
    Unsynced(u8),
    Synced((u8, Vec<u8>)),
    FinalCheck(Vec<u8>),
}

struct HdlcDeframer {
    src: Streamp<u8>,
    dst: Streamp<Vec<u8>>,
    state: State,
    min_size: usize,
    max_size: usize,
}

impl HdlcDeframer {
    /// Create new HdlcDeframer.
    pub fn new(src: Streamp<u8>, min_size: usize, max_size: usize) -> Self {
        Self {
            src,
            dst: new_streamp(),
            min_size,
            max_size,
            state: State::Unsynced(0xff),
        }
    }
    pub fn out(&self) -> Streamp<Vec<u8>> {
        self.dst.clone()
    }

    fn update_state(
        dst: Streamp<Vec<u8>>,
        state: &mut State,
        min_size: usize,
        max_size: usize,
        bit: u8,
    ) -> Result<State> {
        Ok(match state {
            State::Unsynced(v) => {
                let n = (*v >> 1) | (bit << 7);
                if n == 0x7e {
                    debug!("HdlcDeframer: Found flag!");
                    State::Synced((0, Vec::with_capacity(max_size)))
                } else {
                    State::Unsynced(n)
                }
            }
            State::Synced((ones, inbits)) => {
                let mut bits: Vec<u8> = Vec::new();
                std::mem::swap(&mut bits, inbits);
                if inbits.len() > max_size * 8 {
                    return Ok(State::Unsynced(0xff));
                }
                if bit > 0 {
                    bits.push(1);
                    if *ones == 5 {
                        State::FinalCheck(bits)
                    } else {
                        State::Synced((*ones + 1, bits))
                    }
                } else {
                    if *ones == 5 {
                        State::Synced((0, bits))
                    } else {
                        bits.push(0);
                        State::Synced((0, bits))
                    }
                }
            }
            State::FinalCheck(inbits) => {
                let mut bits: Vec<u8> = Vec::new();
                std::mem::swap(&mut bits, inbits);
                if bit == 1 {
                    return Ok(State::Unsynced(0xff));
                }

                bits.push(0);
                bits.truncate(bits.len() - 8);
                if !bits.is_empty() {
                    if bits.len() % 8 != 0 {
                        debug!("HdlcDeframer: Packet len not multiple of 8: {}", bits.len());
                    } else if bits.len() < min_size * 8 {
                        debug!("Packet too short: {} < {}", bits.len() / 8, min_size);
                    } else {
                        let bytes: Vec<u8> = (0..bits.len())
                            .step_by(8)
                            .map(|i| bits2byte(&bits[i..i + 8]))
                            .collect();
                        info!("HdlcDeframer: Captured packet: {:0>2x?}", bytes);

                        // TODO: why do I need to map this? Why do I get a BS compile error when I do:
                        // dst.lock()?.push(bytes);
                        dst.lock()
                            .map_err(|e| Error::new(&format!("bleh: {:?}", e)))?
                            .push(bytes);
                    }
                }
                State::Synced((0, Vec::with_capacity(max_size)))
            }
        })
    }
}

impl Block for HdlcDeframer {
    fn block_name(&self) -> &'static str {
        "HDLC Deframer"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        for bit in input.iter().copied() {
            self.state = Self::update_state(
                self.dst.clone(),
                &mut self.state,
                self.min_size,
                self.max_size,
                bit,
            )?;
        }
        input.clear();
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
        let prev = add_block![g, FileSource::<Complex>::new("aprs-50k.c32", false)?];
        //let prev = add_block![g, FileSource::<Complex>::new("test-50k.c32", false)?];
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

    let prev = add_block![g, HdlcDeframer::new(prev, 10, 1500)];
    g.add(Box::new(PduWriter::new(prev, "packets".to_string())));

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
