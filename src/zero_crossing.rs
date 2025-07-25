//! Very simple clock recovery.
use crate::Result;

use crate::Float;
use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};

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
#[rustradio(crate)]
pub struct ZeroCrossing {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    last_sign: bool,
    last_cross: f32,
    counter: u64,
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Float>,
    // TODO: should this also be tagged `out`?
    out_clock: Option<WriteStream<Float>>,
}

impl ZeroCrossing {
    /** Create new ZeroCrossing block.

    # Args
    * `sps`: Samples per symbol. IOW `samp_rate / baud`.
    * `max_deviation`: Not currently used.
     */
    pub fn new(
        src: ReadStream<Float>,
        sps: Float,
        max_deviation: Float,
    ) -> (Self, ReadStream<Float>) {
        assert!(sps > 1.0);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                src,
                dst,
                sps,
                clock: sps,
                max_deviation,
                last_sign: false,
                last_cross: 0.0,
                counter: 0,
                out_clock: None,
            },
            dr,
        )
    }

    /// Return clock stream.
    #[must_use]
    pub fn out_clock(&mut self) -> ReadStream<Float> {
        assert!(self.out_clock.is_none());
        let (w, r) = crate::stream::new_stream();
        self.out_clock = Some(w);
        r
    }
}

impl Block for ZeroCrossing {
    fn work(&mut self) -> Result<BlockRet> {
        let (input, _tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let mut n = 0;
        let mut opos = 0;
        let mut out_clock = match self.out_clock.as_mut().map(|x| x.write_buf()) {
            None => None,
            Some(Ok(x)) => Some(x),
            Some(Err(e)) => return Err(e),
        };
        let max_out = if let Some(ref clock) = out_clock {
            std::cmp::min(o.len(), clock.len())
        } else {
            o.len()
        };
        for sample in input.iter() {
            n += 1;
            if self.counter == (self.last_cross + (self.clock / 2.0)) as u64 {
                o.slice()[opos] = *sample;
                if let Some(ref mut s) = out_clock {
                    s.slice()[opos] = self.clock;
                }
                opos += 1;
                self.last_cross += self.clock;
                if opos == max_out {
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
        Ok(BlockRet::Again)
    }
}
