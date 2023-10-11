//! Generate a pure signal.
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Stream;
use crate::{Complex, Error, Float};

/// Generate a pure complex sine wave.
pub struct SignalSourceComplex {
    dst: Arc<Mutex<Stream<Complex>>>,

    amplitude: Float,
    rad_per_sample: f64,
    current: f64,
}

/// Generate pure complex sine sine.
impl SignalSourceComplex {
    /// Create new SignalSourceComplex block.
    pub fn new(samp_rate: Float, freq: Float, amplitude: Float) -> Self {
        Self {
            dst: Arc::new(Mutex::new(Stream::new())),
            current: 0.0,
            amplitude,
            rad_per_sample: 2.0 * std::f64::consts::PI * (freq as f64) / (samp_rate as f64),
        }
    }
    pub fn out(&self) -> Arc<Mutex<Stream<Complex>>> {
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
        let n = 1000; // TODO
        let v: Vec<Complex> = self.take(n).collect();
        self.dst.lock().unwrap().write_slice(&v);
        Ok(BlockRet::Ok)
    }
}
