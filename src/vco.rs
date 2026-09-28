//! Voltage Controlled Oscillator.
//!
//! IOW an FM modulator.
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

const MX: f64 = 2.0 * std::f64::consts::PI;

/// Voltage Controlled Oscillator.
///
/// IOW an FM modulator.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Vco {
    #[rustradio(in)]
    src: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,

    k: f64,

    #[rustradio(default)]
    phase: f64,
}

impl Vco {
    fn process_sync(&mut self, a: Float) -> Complex {
        self.phase = (self.phase + self.k * f64::from(a)).rem_euclid(MX);
        Complex::new(self.phase.sin() as Float, self.phase.cos() as Float)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::stream::ReadStream;

    #[test]
    fn large_phase_steps_remain_normalized() -> crate::Result<()> {
        let (mut vco, _output) = Vco::new(ReadStream::from_slice(&[1.0]), MX * 1_000_000.0);
        vco.work()?;
        assert!((0.0..MX).contains(&vco.phase));
        Ok(())
    }
}
