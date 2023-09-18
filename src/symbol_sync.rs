/*
* Much evolving clock sync.
*
* Study material:
* https://youtu.be/jag3btxSsig
*/
use anyhow::Result;

use crate::{Block, Float, StreamReader, StreamWriter};

trait Ted {
    fn error(&self, input: &[Float]) -> Float;
}

struct TedDeriv {}

impl TedDeriv {
    fn new() -> Self {
        Self {}
    }
}

impl Ted for TedDeriv {
    fn error(&self, input: &[Float]) -> Float {
        let sign = if input[0] > 0.0 { 1.0 } else { -1.0 };
        sign * input[input.len() - 1] - input[0]
    }
}

pub struct SymbolSync {
    _sps: Float,
    _max_deviation: Float,
    clock: Float,
    ted: Box<dyn Ted>,
}

impl SymbolSync {
    pub fn new(sps: Float, max_deviation: Float) -> Self {
        assert!(sps > 1.0);
        Self {
            _sps: sps,
            _max_deviation: max_deviation,
            clock: sps,
            ted: Box::new(TedDeriv::new()),
        }
    }
}

/*
error = sign(x) * deriv(x)
positive error means "early", neagive error means "late"
*/

impl Block<Float, Float> for SymbolSync {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Float>,
        w: &mut dyn StreamWriter<Float>,
    ) -> Result<()> {
        let mut v = Vec::new();
        let n = r.buffer().len();
        let mut pos = Float::default();
        loop {
            let i = pos as usize;
            if i + 1 >= n {
                break;
            }
            let error = self.ted.error(&r.buffer()[i..i + 1]);
            if error > 0.0 {
                pos += 0.3;
            } else {
                pos -= 0.3;
            }
            v.push(r.buffer()[i]);
            pos += self.clock;
        }
        r.consume(n);
        w.write(&v)
    }
}

pub struct ZeroCrossing {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    last_sign: bool,
    last_cross: u64,
    counter: u64,
}

impl ZeroCrossing {
    pub fn new(sps: Float, max_deviation: Float) -> Self {
        assert!(sps > 1.0);
        Self {
            sps,
            clock: sps,
            max_deviation,
            last_sign: false,
            last_cross: 0,
            counter: 0,
        }
    }
}

impl Block<Float, Float> for ZeroCrossing {
    fn work(
        &mut self,
        r: &mut dyn StreamReader<Float>,
        w: &mut dyn StreamWriter<Float>,
    ) -> Result<()> {
        let mut v = Vec::new();
        for sample in r.buffer().iter() {
            if self.counter == self.last_cross + (self.clock / 2.0) as u64 {
                v.push(*sample);
                self.last_cross += self.clock as u64;
            }

            let sign = *sample > 0.0;
            if sign != self.last_sign {
                self.last_cross = self.counter;
                // TODO: adjust clock, within sps. Here just shut up the linter.
                self.sps *= 1.0;
                self.max_deviation *= 1.0;
            }
            self.last_sign = sign;
            self.counter += 1;
        }
        r.consume(r.buffer().len());
        w.write(&v)
    }
}
