//! Clock recovery implementations
/*
* Study material:
* https://youtu.be/jag3btxSsig
* https://youtu.be/uMEfx_l5Oxk
*/
use anyhow::Result;
use log::{debug, info, trace};
use std::collections::VecDeque;

use crate::block::{Block, BlockRet};
use crate::single_pole_iir_filter::SinglePoleIIR;
use crate::stream::{new_streamp, Streamp};
use crate::{Error, Float};

/** Very simple clock recovery by looking at zero crossings.

Every time the stream crosses 0, this is assumed to be right in the
middle of two symbols, and the next chosen sample to use as a symbol
will be the one `sps/2` samples later.

The one after that will be after `1.5*sps` samples. And so on, until
the next zero crossing happens, and the clock thus resets.

Future work in this block will be to adjust the sps according to when
the expected vs actual zero crossings happen, effectively phase lock
looping.

But for now it's "good enough" to get simple 2FSK decoded pretty
reliably.
 */
pub struct ZeroCrossing {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    clock_filter: SinglePoleIIR<Float>,
    last_sign: bool,
    stream_pos: Float,
    last_sym_boundary_pos: Float,
    next_sym_middle: Float,
    crossing_history: VecDeque<Float>,
    src: Streamp<Float>,
    dst: Streamp<Float>,
    out_clock: Option<Streamp<Float>>,
}

impl ZeroCrossing {
    /** Create new ZeroCrossing block.

    # Args
    * `sps`: Samples per symbol. IOW `samp_rate / baud`.
    * `max_deviation`: Not currently used.
     */
    pub fn new(src: Streamp<Float>, sps: Float, max_deviation: Float) -> Self {
        assert!(sps > 1.0);
        let mut clock_filter = SinglePoleIIR::new(0.01).unwrap();
        clock_filter.set_prev(sps);
        Self {
            src,
            dst: new_streamp(),
            sps,
            clock: sps,
            clock_filter,
            max_deviation,
            last_sign: false,
            stream_pos: 0.0,
            last_sym_boundary_pos: 0.0,
            next_sym_middle: 0.0,
            out_clock: None,
            crossing_history: VecDeque::new(),
        }
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<Float> {
        self.dst.clone()
    }

    /// Return the output stream.
    pub fn out_clock(&mut self) -> Streamp<Float> {
        let r = self.out_clock.get_or_insert(new_streamp()).clone();
        r
    }
}

impl Block for ZeroCrossing {
    fn block_name(&self) -> &'static str {
        "ZeroCrossing"
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
                    if self.stream_pos > 0.0 {
                        if true && t > mi * 0.8 && t < mx * 1.2 {
                            // Single pole IIR
                            assert!(
                                t > 0.0,
                                "t negative {} {}",
                                self.stream_pos,
                                self.last_sym_boundary_pos
                            );
                            self.clock = self.clock_filter.filter_capped(t, mi, mx);
                            self.next_sym_middle = self.last_sym_boundary_pos + self.clock / 2.0;
                            while self.next_sym_middle < self.stream_pos {
                                self.next_sym_middle += self.clock;
                            }
                            debug!(
                                "ZeroCrossing: clock@{} pre={pre} now={t} min={mi} max={mx} => {}",
                                self.stream_pos, self.clock
                            );
                        } else if false && t > mi / 2.0 {
                            // FIR.
                            self.crossing_history.push_back(t);
                            if self.crossing_history.len() > 5 {
                                self.crossing_history.pop_front();
                            }
                            let sum: Float = self.crossing_history.iter().sum();
                            let t = (sum / self.crossing_history.len() as Float).max(mi).min(mx);
                            self.clock = self.clock_filter.filter_capped(t, mi, mx);
                            debug!("{:?}", self.crossing_history);
                            debug!(
                                "ZeroCrossing: clock@{} pre={pre} now={t} min={mi} max={mx} => {}",
                                self.stream_pos, self.clock
                            );
                        }
                    }
                }
                self.last_sym_boundary_pos = self.stream_pos;
                self.last_sign = sign;
            }
            self.stream_pos += 1.0;

            // Stay around zero so that we don't lose float precision.
            if true {
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
        }
        input.consume(n);
        o.produce(opos, &[]);
        if let Some(s) = out_clock {
            s.produce(opos, &[]);
        }
        Ok(BlockRet::Ok)
    }
}
