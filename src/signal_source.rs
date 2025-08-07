//! Generate a pure signal.
use crate::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Generate a pure complex sine wave.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SignalSourceComplex {
    #[rustradio(out)]
    dst: WriteStream<Complex>,

    amplitude: Float,
    rad_per_sample: f64,
    current: f64,
}

/// Generate pure complex sine sine.
impl SignalSourceComplex {
    /// Create new SignalSourceComplex block.
    pub fn new(samp_rate: Float, freq: Float, amplitude: Float) -> (Self, ReadStream<Complex>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                dst,
                current: 0.0,
                amplitude,
                rad_per_sample: 2.0 * std::f64::consts::PI * (freq as f64) / (samp_rate as f64),
            },
            dr,
        )
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
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut o = self.dst.write_buf()?;
        let n = o.len();
        for (to, from) in o.slice().iter_mut().zip(self.take(n)) {
            *to = from;
        }
        o.produce(n, &[]);
        Ok(BlockRet::Again)
    }
}

/// Generate a pure real sine wave.
///
/// TODO: not an efficient implementation, and duplicates code with the Complex
/// version.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SignalSourceFloat {
    #[rustradio(out)]
    dst: WriteStream<Float>,
    amplitude: Float,
    rad_per_sample: f64,
    current: f64,
}

/// Generate pure complex sine sine.
impl SignalSourceFloat {
    /// Create new SignalSourceFloat block.
    pub fn new(samp_rate: Float, freq: Float, amplitude: Float) -> (Self, ReadStream<Float>) {
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                dst,
                current: 0.0,
                amplitude,
                rad_per_sample: 2.0 * std::f64::consts::PI * (freq as f64) / (samp_rate as f64),
            },
            dr,
        )
    }
}

impl Iterator for SignalSourceFloat {
    type Item = Float;
    fn next(&mut self) -> Option<Float> {
        self.current = (self.current + self.rad_per_sample) % (2.0 * std::f64::consts::PI);
        Some(self.amplitude * self.current.sin() as Float)
    }
}

impl Block for SignalSourceFloat {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut o = self.dst.write_buf()?;
        let n = o.len();
        o.slice()
            .iter_mut()
            .zip(self)
            .map(|(to, from)| {
                *to = from;
            })
            .for_each(drop);
        o.produce(n, &[]);
        Ok(BlockRet::Again)
    }
}
/* vim: textwidth=80
 */
