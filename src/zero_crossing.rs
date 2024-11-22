//! Very simple clock recovery.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{Stream, Streamp};
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
#[derive(rustradio_macros::Block)]
#[rustradio(crate, out)]
pub struct ZeroCrossing {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    last_sign: bool,
    last_cross: f32,
    counter: u64,
    #[rustradio(in)]
    src: Streamp<Float>,
    #[rustradio(out)]
    dst: Streamp<Float>,
    // TODO: should this also be tagged `out`?
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
        Self {
            src,
            dst: Stream::newp(),
            sps,
            clock: sps,
            max_deviation,
            last_sign: false,
            last_cross: 0.0,
            counter: 0,
            out_clock: None,
        }
    }

    /// Return clock stream.
    pub fn out_clock(&mut self) -> Streamp<Float> {
        self.out_clock.get_or_insert(Stream::newp()).clone()
    }
}

impl Block for ZeroCrossing {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (input, _tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut n = 0;
        let mut opos = 0;
        // TODO: get rid of unwrap.
        let mut out_clock = self.out_clock.as_mut().map(|x| x.write_buf().unwrap());
        for sample in input.iter() {
            n += 1;
            if self.counter == (self.last_cross + (self.clock / 2.0)) as u64 {
                o.slice()[opos] = *sample;
                if let Some(ref mut s) = out_clock {
                    s.slice()[opos] = self.clock;
                }
                opos += 1;
                self.last_cross += self.clock;
                if opos == o.len() {
                    break;
                }
            }

            let sign = *sample > 0.0;
            if sign != self.last_sign {
                self.last_cross = self.counter as f32;
                // TODO: adjust clock, within sps. Here just shut up the linter.
                self.sps *= 1.0;
                self.max_deviation *= 1.0;
            }
            self.last_sign = sign;
            self.counter += 1;

            let step_back = (10.0 * self.clock) as u64;
            if self.counter > step_back && self.last_cross as u64 > step_back {
                self.counter -= step_back;
                self.last_cross -= step_back as f32;
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
