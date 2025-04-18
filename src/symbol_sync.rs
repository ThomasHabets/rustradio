//! Clock recovery implementations
/*
* Study material:
* https://youtu.be/jag3btxSsig
* https://youtu.be/uMEfx_l5Oxk
*/
use log::{trace, warn};

use crate::block::{Block, BlockRet};
use crate::iir_filter::ClampedFilter;
use crate::stream::{ReadStream, WriteStream};
use crate::{Float, Result};

/// Timing Error Detector.
pub trait Ted: Send {}

/// ZeroCrossing TED.
pub struct TedZeroCrossing {}

impl TedZeroCrossing {
    /// Create new TED.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for TedZeroCrossing {
    fn default() -> Self {
        Self::new()
    }
}

// Dummy implementation. Zerocrossing is actually the only
// implementation.
impl Ted for TedZeroCrossing {}

/** Pluggable clock recovery block.

Under development.

TODO: implement real EOF handling.
*/
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SymbolSync {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    _ted: Box<dyn Ted>,
    clock_filter: Box<dyn ClampedFilter<Float>>,
    last_sign: bool,
    stream_pos: Float,
    last_sym_boundary_pos: Float,
    next_sym_middle: Float,
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
    #[rustradio(out)]
    out_clock: Option<WriteStream<Float>>,
}

impl SymbolSync {
    /** Create new SymbolSync block.

    # Args
    * `sps`: Samples per symbol. IOW `samp_rate / baud`.
     */
    pub fn new(
        src: ReadStream<Float>,
        sps: Float,
        max_deviation: Float,
        ted: Box<dyn Ted>,
        mut clock_filter: Box<dyn ClampedFilter<Float>>,
    ) -> (Self, ReadStream<Float>) {
        assert!(sps > 1.0);
        clock_filter.fill(sps);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
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
            },
            dr,
        )
    }

    /// Return clock stream.
    ///
    /// The output stream can only be created once, so if called a second time,
    /// just returns None.
    pub fn out_clock(&mut self) -> Option<ReadStream<Float>> {
        if self.out_clock.is_some() {
            warn!("SymbolSync::out_clock() called more than once");
            return None;
        }
        let (tx, rx) = crate::stream::new_stream();
        self.out_clock = Some(tx);
        Some(rx)
    }
}

impl Block for SymbolSync {
    fn work(&mut self) -> Result<BlockRet> {
        let (input, _tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
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
                        self.clock = self.clock_filter.filter_clamped(
                            t - self.sps,
                            mi - self.sps,
                            mx - self.sps,
                        ) + self.sps;
                        self.next_sym_middle = self.last_sym_boundary_pos + self.clock / 2.0;
                        while self.next_sym_middle < self.stream_pos {
                            self.next_sym_middle += self.clock;
                        }
                        trace!(
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
        Ok(BlockRet::Again)
    }
}
/* vim: textwidth=80
 */
