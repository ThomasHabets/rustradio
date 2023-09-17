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
