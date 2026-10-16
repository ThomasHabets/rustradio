//! Frequency translation through complex exponential.

use crate::AlgebraicMul;
use crate::signal_source::SignalSourceComplex;
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float, Result};

/// Frequency translate through complex exponential.
///
/// This does not filter, but just "rotates" the frequency spectrum within the
/// nyquist.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct Rotator {
    translator: SignalSourceComplex,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl Rotator {
    /// Create rotator from sample rate and frequency.
    pub fn new(
        src: ReadStream<Complex>,
        samp_rate: Float,
        freq: Float,
    ) -> Result<(Self, ReadStream<Complex>)> {
        let (xlat, _) = SignalSourceComplex::new(samp_rate, freq, 1.0)?;
        let (dst, dr) = crate::stream::new_stream()?;
        Ok((
            Self {
                translator: xlat,
                dst,
                src,
            },
            dr,
        ))
    }

    fn process_sync(&mut self, x: Complex) -> Complex {
        x.algebraic_mul(
            self.translator
                .next()
                .expect("can't happen: infinite stream ended"),
        )
    }
}
