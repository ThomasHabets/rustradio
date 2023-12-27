//! Clock recovery implementations
/*
* Study material:
* https://youtu.be/jag3btxSsig
* https://youtu.be/uMEfx_l5Oxk
*/
use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::iir_filter::CappedFilter;
use crate::stream::{new_streamp, Streamp};
use crate::{Error, Float};

/// Timing Error Detector.
pub trait TED {}

/// ZeroCrossing TED.
pub struct TEDZeroCrossing {}

impl TEDZeroCrossing {
    /// Create new TED.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for TEDZeroCrossing {
    fn default() -> Self {
        Self::new()
    }
}

// Dummy implementation. Zerocrossing is actually the only
// implementation.
impl TED for TEDZeroCrossing {}

/** Pluggable clock recovery block.

Under development.
*/
pub struct SymbolSync {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    _ted: Box<dyn TED>,
    clock_filter: Box<dyn CappedFilter<Float>>,
    last_sign: bool,
    stream_pos: Float,
    last_sym_boundary_pos: Float,
    next_sym_middle: Float,
    src: Streamp<Float>,
    dst: Streamp<Float>,
    out_clock: Option<Streamp<Float>>,
}

impl SymbolSync {
    /** Create new SymbolSync block.

    # Args
    * `sps`: Samples per symbol. IOW `samp_rate / baud`.
     */
    pub fn new(
        src: Streamp<Float>,
        sps: Float,
        max_deviation: Float,
        ted: Box<dyn TED>,
        mut clock_filter: Box<dyn CappedFilter<Float>>,
    ) -> Self {
        assert!(sps > 1.0);
        clock_filter.fill(sps);
        Self {
            src,
            dst: new_streamp(),
            sps,
            clock: sps,
            _ted: ted,
            clock_filter,
            max_deviation,
            last_sign: false,
            stream_pos: 0.0,
            last_sym_boundary_pos: 0.0,
            next_sym_middle: 0.0,
            out_clock: None,
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<Float> {
        self.dst.clone()
    }

    /// Return clock stream.
    pub fn out_clock(&mut self) -> Streamp<Float> {
        self.out_clock.get_or_insert(new_streamp()).clone()
    }
}

impl Block for SymbolSync {
    fn block_name(&self) -> &'static str {
        "SymbolSync"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (input, _tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::Noop);
        }
        // TODO: get rid of unwrap.
        let mut out_clock = self.out_clock.as_mut().map(|x| x.write_buf().unwrap());

        let mut n = 0; // Samples consumed.
        let mut opos = 0; // Current output position.
        let olen = o.len();
        let oslice = o.slice();
        for sample in input.iter() {
            n += 1;
            if self.stream_pos >= self.next_sym_middle {
                // TODO: use more than center sample.
                oslice[opos] = *sample;
                if let Some(ref mut s) = out_clock {
                    s.slice()[opos] = self.clock;
                }
                opos += 1;
                self.next_sym_middle += self.clock;
                if opos == olen {
                    break;
                }
            }
            let sign = *sample > 0.0;
            if sign != self.last_sign {
                if self.stream_pos > 0.0 && self.last_sym_boundary_pos > 0.0 {
                    assert!(
                        self.stream_pos > self.last_sym_boundary_pos,
                        "{} not > {}",
                        self.stream_pos,
                        self.last_sym_boundary_pos
                    );
                    let mi = self.sps - self.max_deviation;
                    let mx = self.sps + self.max_deviation;
                    let mut t = self.stream_pos - self.last_sym_boundary_pos;
                    let pre = self.clock;
                    while t > mx {
                        let t2 = t - self.clock;
                        if (t - self.clock).abs() < (t2 - self.clock).abs() {
                            break;
                        }
                        t = t2;
                    }
                    if t > mi * 0.8 && t < mx * 1.2 {
                        assert!(
                            t > 0.0,
                            "t negative {} {}",
                            self.stream_pos,
                            self.last_sym_boundary_pos
                        );
                        self.clock = self.clock_filter.filter_capped(
                            t - self.sps,
                            mi - self.sps,
                            mx - self.sps,
                        ) + self.sps;
                        self.next_sym_middle = self.last_sym_boundary_pos + self.clock / 2.0;
                        while self.next_sym_middle < self.stream_pos {
                            self.next_sym_middle += self.clock;
                        }
                        debug!(
                            "SymbolSync: clock@{} pre={pre} now={t} min={mi} max={mx} => {}",
                            self.stream_pos, self.clock
                        );
                    }
                }
                self.last_sym_boundary_pos = self.stream_pos;
                self.last_sign = sign;
            }
            self.stream_pos += 1.0;
            // Stay around zero so that we don't lose float precision.
            let step_back = 10.0 * self.clock;
            if self.stream_pos > step_back
                && self.last_sym_boundary_pos > step_back
                && self.next_sym_middle > step_back
            {
                self.stream_pos -= step_back;
                self.last_sym_boundary_pos -= step_back;
                self.next_sym_middle -= step_back;
            }
        }
        input.consume(n);
        o.produce(opos, &[]);
        if let Some(s) = out_clock {
            s.produce(opos, &[]);
        }
        Ok(BlockRet::Ok)
    }
}
