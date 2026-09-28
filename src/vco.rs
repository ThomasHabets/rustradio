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
        Complex::new(self.phase.cos() as Float, self.phase.sin() as Float)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::stream::ReadStream;
    use std::f64::consts::PI;

    fn phase_close(mut a: f64, mut b: f64) -> bool {
        const E: f64 = 0.0001;
        while a < 0.0 {
            a += MX;
        }
        while b < 0.0 {
            b += MX;
        }
        a = a.rem_euclid(MX);
        b = b.rem_euclid(MX);
        if (a - b).abs() < E {
            true
        } else {
            (MX - (a - b).abs()).abs() < E
        }
    }

    fn is_close(a: &[f64], b: &[f64]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        for (x, y) in a.iter().zip(b.iter()) {
            if !phase_close(*x, *y) {
                return false;
            }
        }
        true
    }

    #[test]
    fn large_phase_steps_remain_normalized() -> crate::Result<()> {
        let (mut vco, _output) = Vco::new(ReadStream::from_slice(&[1.0]), MX * 1_000_000.0);
        vco.work()?;
        assert!(phase_close(vco.phase, 0.0), "phase {} not right", vco.phase);
        Ok(())
    }

    #[test]
    fn positive() -> crate::Result<()> {
        let (mut vco, output) = Vco::new(
            ReadStream::from_slice(&[1.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
            PI / 2.0,
        );
        vco.work()?;
        let (o, _tags) = output.read_buf()?;
        assert!(o.iter().map(|c| c.norm()).all(|m| (m - 1.0).abs() < 0.0001));
        let phases: Vec<_> = o.iter().map(|c| c.arg() as f64).collect();
        let want = &[PI / 2.0, PI / 2.0, PI, 1.5 * PI, 0.0, PI / 2.0];
        assert!(
            is_close(phases.as_ref(), want),
            "got  {phases:?}\nwant {want:?}"
        );
        assert!((0.0..MX).contains(&vco.phase));
        assert_eq!(vco.phase, std::f64::consts::PI / 2.0);
        Ok(())
    }

    #[test]
    fn negative() -> crate::Result<()> {
        let (mut vco, output) = Vco::new(
            ReadStream::from_slice(&[-1.0, 0.0, -0.5, -0.5, -2.0, -0.5]),
            PI / 2.0,
        );
        vco.work()?;
        let (o, _tags) = output.read_buf()?;
        assert!(o.iter().map(|c| c.norm()).all(|m| (m - 1.0).abs() < 0.0001));
        let phases: Vec<_> = o.iter().map(|c| c.arg() as f64).collect();
        let want = &[
            -PI / 2.0,
            -PI / 2.0,
            -1.5 * PI / 2.0,
            -PI, // Equivalently, PI.
            0.0,
            -PI / 4.0,
        ];
        assert!(
            is_close(phases.as_ref(), want),
            "got  {phases:?}\nwant {want:?}"
        );
        assert!(
            phase_close(vco.phase, 2.0 * PI - PI / 4.0),
            "phase {} not right",
            vco.phase
        );
        Ok(())
    }
}
