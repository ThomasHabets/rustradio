/*
* Much evolving clock sync.
*
* Study material:
* https://youtu.be/jag3btxSsig
*/
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams};
use crate::{Error, Float};

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

impl Block for SymbolSync {
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let input = Self::get_input::<Float>(r, 0);
        let mut v = Vec::new();
        let n = input.borrow().available();
        let mut pos = Float::default();
        loop {
            let i = pos as usize;
            if i + 1 >= n {
                break;
            }
            // TODO: needless copy.
            let t: Vec<Float> = input.borrow().data().clone().into();
            let error = self.ted.error(&t);
            if error > 0.0 {
                pos += 0.3;
            } else {
                pos -= 0.3;
            }
            v.push(input.borrow().data()[i]);
            pos += self.clock;
        }
        input.borrow_mut().clear();
        Self::get_output::<Float>(w, 0).borrow_mut().write_slice(&v);
        Ok(BlockRet::Ok)
    }
}

pub struct ZeroCrossing {
    sps: Float,
    max_deviation: Float,
    clock: Float,
    last_sign: bool,
    last_cross: f32,
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
            last_cross: 0.0,
            counter: 0,
        }
    }
}

impl Block for ZeroCrossing {
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let mut v = Vec::new();
        let input = Self::get_input(r, 0);
        for sample in input.borrow().iter() {
            if self.counter == (self.last_cross + (self.clock / 2.0)) as u64 {
                v.push(*sample);
                self.last_cross += self.clock;
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
        input.borrow_mut().clear();
        Self::get_output::<Float>(w, 0).borrow_mut().write_slice(&v);
        Ok(BlockRet::Ok)
    }
}
