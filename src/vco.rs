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
        self.phase += self.k * (a as f64);
        if self.phase > MX {
            self.phase -= MX;
        }
        if self.phase < -MX {
            self.phase += MX;
        }
        Complex::new(self.phase.sin() as Float, self.phase.cos() as Float)
    }
}
