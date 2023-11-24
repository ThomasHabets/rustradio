//! Generate a pure signal.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp, ReadStreamp};
use crate::{Complex, Error, Float};

/// Generate a pure complex sine wave.
pub struct SignalSourceComplex {
    dst: Streamp<Complex>,

    amplitude: Float,
    rad_per_sample: f64,
    current: f64,
}

/// Generate pure complex sine sine.
impl SignalSourceComplex {
    /// Create new SignalSourceComplex block.
    pub fn new(samp_rate: Float, freq: Float, amplitude: Float) -> Self {
        Self {
            dst: new_streamp(),
            current: 0.0,
            amplitude,
            rad_per_sample: 2.0 * std::f64::consts::PI * (freq as f64) / (samp_rate as f64),
        }
    }
    /// Return the output stream.
    pub fn out(&self) -> Streamp<Complex> {
        self.dst.clone()
    }
}

impl Iterator for SignalSourceComplex {
    type Item = Complex;
    fn next(&mut self) -> Option<Complex> {
        self.current = (self.current + self.rad_per_sample) % (2.0 * std::f64::consts::PI);
        Some(
            self.amplitude
                * Complex::new(
                    self.current.sin() as Float,
                    (self.current - std::f64::consts::PI / 2.0).sin() as Float,
                ),
        )
    }
}

impl Block for SignalSourceComplex {
    fn block_name(&self) -> &'static str {
        "SignalSourceComplex"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let obind = self.dst.clone();
        let mut o = obind.write_buf()?;
        let n = o.len();
        for (to, from) in o.slice().iter_mut().zip(self.take(n)) {
            *to = from;
        }
        o.produce(n, &[]);
        Ok(BlockRet::Ok)
    }
}
