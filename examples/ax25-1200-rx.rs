/*! AX.25 1200bps Bell 202 receiver.

Can be used to receive APRS over the air with RTL-SDR or from
complex I/Q saved to a file.

```no_run
$ mkdir captured
$ ./ax25-1200-rx -r captured.c32 --samp_rate 50000 -o captured
[…]
$ ./ax25-1200-rx --rtlsdr -o captured -v 2
[…]
```
*/
use std::io::Write;
use std::time::SystemTime;

use anyhow::Result;
use log::{debug, info};
use structopt::StructOpt;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::graph::Graph;
use rustradio::stream::{new_streamp, Streamp};
use rustradio::{Complex, Error, Float};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    #[structopt(long = "out", short = "o")]
    output: String,

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

/** PDU writer

This block takes PDUs (as Vec<u8>), and writes them to an output
directory, named as microseconds since epoch.
*/
pub struct PduWriter {
    src: Streamp<Vec<u8>>,
    dir: String,
}

impl PduWriter {
    /// Create new PduWriter that'll write to `dir`.
    pub fn new(src: Streamp<Vec<u8>>, dir: String) -> Self {
        Self { src, dir }
    }
}

impl Block for PduWriter {
    fn block_name(&self) -> &'static str {
        "PDU Writer"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut input = self.src.lock()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        for packet in input.iter() {
            let mut f = std::fs::File::create(format!(
                "{}/{}",
                self.dir,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_micros()
            ))?;
            f.write_all(packet)?;
        }
        input.clear();
        Ok(BlockRet::Ok)
    }
}

enum State {
    /// Looking for flag pattern.
    Unsynced(u8),

    /// Flag pattern seen. Accumulating bits for packet.
    Synced((u8, Vec<u8>)),

    /// Six ones in a row seen. Check the final bit for a 0, and emit
    /// packet if so.
    FinalCheck(Vec<u8>),
}

/** HDLC Deframer.

This block takes a stream of bits (as u8), and outputs any HDLC frames
found as Vec<u8>.

TODO: Check checksum, and only output packets that pass.
*/
pub struct HdlcDeframer {
    src: Streamp<u8>,
    dst: Streamp<Vec<u8>>,
    state: State,
    min_size: usize,
    max_size: usize,
}

impl HdlcDeframer {
    /// Create new HdlcDeframer.
    ///
    /// min_size and max_size is size in bytes.
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
                // We can't move from `bits`, since it's only borrowed,
                // but we can swap its contents.
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
                // We can't move from `bits`, since it's only borrowed,
                // but we can swap its contents.
                std::mem::swap(&mut bits, inbits);
                if bit == 1 {
                    // 7 ones in a row is invalid. Discard what we've collected.
                    return Ok(State::Unsynced(0xff));
                }
                if bits.len() < 7 {
                    // Too short, not even zero bytes.
                    return Ok(State::Unsynced(0xff));
                }

                // Remove partial flag.
                bits.truncate(bits.len() - 7);

                if bits.len() % 8 != 0 {
                    debug!("HdlcDeframer: Packet len not multiple of 8: {}", bits.len());
                } else if bits.len() / 8 < min_size {
                    debug!("Packet too short: {} < {}", bits.len() / 8, min_size);
                } else {
                    let bytes: Vec<u8> = (0..bits.len())
                        .step_by(8)
                        .map(|i| bits2byte(&bits[i..i + 8]))
                        .collect();
                    info!("HdlcDeframer: Captured packet: {:0>2x?}", bytes);

                    // TODO: why do I need to map this? Why do I get a
                    // BS compile error when I do:
                    //
                    // dst.lock()?.push(bytes);
                    dst.lock()
                        .map_err(|e| Error::new(&format!("not possible?: {:?}", e)))?
                        .push(bytes);
                }

                // We may or may not have seen a valid packet, but we
                // did see a valid flag. So back to synced.
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

// Turn 8 bits in LSB order into a byte.
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
    g.add(Box::new(PduWriter::new(prev, opt.output)));

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
