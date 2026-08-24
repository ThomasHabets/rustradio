//! Automatic gain control for complex samples.
//!
//! [`Agc`] scales each input sample using a feedback-controlled gain so that
//! the output magnitude approaches a configurable reference. Gain decreases
//! at the attack rate when the output is too large and increases at the decay
//! rate when it is too small.

use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Float, Result};

/// Builder for [`Agc`].
#[derive(Clone, Debug)]
#[must_use]
pub struct AgcBuilder {
    attack_rate: Float,
    decay_rate: Float,
    reference: Float,
    initial_gain: Float,
    max_gain: Option<Float>,
}

impl AgcBuilder {
    /// Set the target output magnitude. The default is one.
    pub fn reference(mut self, reference: Float) -> Self {
        self.reference = reference;
        self
    }

    /// Set the gain applied to the first sample. The default is one.
    pub fn initial_gain(mut self, gain: Float) -> Self {
        self.initial_gain = gain;
        self
    }

    /// Limit the feedback-controlled gain. By default it is unlimited.
    pub fn max_gain(mut self, gain: Float) -> Self {
        self.max_gain = Some(gain);
        self
    }

    /// Validate the configuration and build an AGC connected to `src`.
    ///
    /// # Errors
    ///
    /// Returns an error if either rate is not finite and between zero and one,
    /// if the reference or initial gain is not finite and positive, or if a
    /// maximum gain is invalid or smaller than the initial gain.
    pub fn build(self, src: ReadStream<Complex>) -> Result<(Agc, ReadStream<Complex>)> {
        validate_rate("attack", self.attack_rate)?;
        validate_rate("decay", self.decay_rate)?;
        validate_positive("reference", self.reference)?;
        validate_positive("initial gain", self.initial_gain)?;
        if let Some(max_gain) = self.max_gain {
            validate_positive("maximum gain", max_gain)?;
            if max_gain < self.initial_gain {
                return Err(Error::msg(
                    "AGC maximum gain must not be less than the initial gain",
                ));
            }
        }

        let (dst, output) = crate::stream::new_stream();
        Ok((
            Agc {
                src,
                dst,
                attack_rate: self.attack_rate,
                decay_rate: self.decay_rate,
                reference: self.reference,
                gain: self.initial_gain,
                max_gain: self.max_gain,
            },
            output,
        ))
    }
}

/// Scale complex samples toward a reference magnitude with feedback AGC.
///
/// This is a one-input, one-output synchronous block, so input tags are
/// propagated unchanged. The current gain is applied before the feedback loop
/// updates it for the next sample. When the scaled magnitude exceeds the
/// reference, `attack_rate` controls how quickly gain falls; otherwise,
/// `decay_rate` controls how quickly gain rises.
///
/// Both rates are coefficients in the inclusive range zero through one. A
/// maximum gain is optional, but is useful when long periods of silence are
/// expected.
///
/// ## Selecting good values
///
/// A useful first approximation for a rate is:
///
/// `r = 1 - e^(-1/Fs*T)`, or `1/(Fs * t)`
///
/// * `r` is the rate (attack or decay rate).
/// * `t` is how fast it should "mostly converge".
/// * `Fs` is the same rate.
///
/// So an attack rate of 0.2 at 125ksps should mostly converge in
/// `1/(0.2*125e3)=40us`. For a strong nearby signal with little else
/// overpowering it, that could work well for OOK or short burst packets.
///
/// For the receiver to put its ear to the ground a decay rate of 1e-4 with the
/// same formula gives 80ms. A common starting point for decay rate is 10 to
/// 100x attack rate, though in this example it was 2000.
///
/// For longer bursts with preambles, or QAM or other multi-amplitude
/// modulations, the rates will likely need to be dialed in to get an even
/// signal, and yet not modify individual symbols or levels across symbols.
///
/// FM/FSK and constant envelope PSK can usually tolerate fast AGC. In fact,
/// forcing to fixed magnitude it's often part of demod.
///
/// For extra sensitivity you may want to feed back the signal strength into the
/// SDR hardware input gain instead, since this AGC block can only work with the
/// bits that it has. An SDR input gain will likely happen before the ADC, thus
/// lose less precision.
///
/// ## TODO
///
/// It may be best to turn off AGC after the preamble. This block should support
/// that through tags in the stream.
///
/// # Example
///
/// ```
/// use rustradio::blocks::{Agc, VectorSource};
/// use rustradio::Complex;
///
/// let (_source, input) = VectorSource::new(vec![Complex::new(0.25, 0.0)]);
/// let (_agc, _output) = Agc::builder(0.1, 0.001)
///     .reference(1.0)
///     .max_gain(100.0)
///     .build(input)?;
/// # Ok::<(), rustradio::Error>(())
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, sync)]
pub struct Agc {
    attack_rate: Float,
    decay_rate: Float,
    reference: Float,
    gain: Float,
    max_gain: Option<Float>,

    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl Agc {
    /// Create an AGC with reference magnitude and initial gain equal to one.
    ///
    /// Use [`Agc::builder`] to configure those values or a maximum gain.
    ///
    /// # Errors
    ///
    /// Returns an error unless both rates are finite and between zero and one.
    pub fn new(
        src: ReadStream<Complex>,
        attack_rate: Float,
        decay_rate: Float,
    ) -> Result<(Self, ReadStream<Complex>)> {
        Self::builder(attack_rate, decay_rate).build(src)
    }

    /// Create a builder with the required attack and decay rates.
    pub fn builder(attack_rate: Float, decay_rate: Float) -> AgcBuilder {
        AgcBuilder {
            attack_rate,
            decay_rate,
            reference: 1.0,
            initial_gain: 1.0,
            max_gain: None,
        }
    }

    fn process_sync(&mut self, input: Complex) -> Complex {
        let output = Complex::new(
            input.re.algebraic_mul(self.gain),
            input.im.algebraic_mul(self.gain),
        );
        let magnitude = output
            .re
            .algebraic_mul(output.re)
            .algebraic_add(output.im.algebraic_mul(output.im))
            .sqrt();
        let error = magnitude.algebraic_add(-self.reference);
        let rate = if error > 0.0 {
            self.attack_rate
        } else {
            self.decay_rate
        };
        self.gain = self
            .gain
            .algebraic_add((-error).algebraic_mul(rate))
            .max(0.0);
        if let Some(max_gain) = self.max_gain {
            self.gain = self.gain.min(max_gain);
        }
        output
    }
}

fn validate_rate(name: &str, rate: Float) -> Result<()> {
    if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
        return Err(Error::msg(format!(
            "AGC {name} rate must be finite and between zero and one"
        )));
    }
    Ok(())
}

fn validate_positive(name: &str, value: Float) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        return Err(Error::msg(format!(
            "AGC {name} must be finite and greater than zero"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::blocks::VectorSource;
    use crate::stream::{Tag, TagValue};

    fn run(input: &[Complex], builder: AgcBuilder) -> Result<Vec<Complex>> {
        let input = ReadStream::from_slice(input);
        let (mut agc, output) = builder.build(input)?;
        agc.work()?;
        let (buffer, _) = output.read_buf()?;
        Ok(buffer.slice().to_vec())
    }

    /// Verify attack and decay select independent update coefficients.
    #[test]
    fn separate_attack_and_decay_rates() -> Result<()> {
        let output = run(
            &[
                Complex::new(2.0, 0.0),
                Complex::new(1.0, 0.0),
                Complex::new(1.0, 0.0),
            ],
            Agc::builder(0.5, 0.25),
        )?;
        assert_eq!(
            output,
            vec![
                Complex::new(2.0, 0.0),
                Complex::new(0.5, 0.0),
                Complex::new(0.625, 0.0),
            ]
        );
        Ok(())
    }

    /// Verify gain scales both complex components without changing phase.
    #[test]
    fn scales_complex_samples() -> Result<()> {
        let output = run(
            &[Complex::new(1.0, -2.0)],
            Agc::builder(0.0, 0.0).initial_gain(2.0),
        )?;
        assert_eq!(output, vec![Complex::new(2.0, -4.0)]);
        Ok(())
    }

    /// Verify configured gain bounds are enforced by the feedback loop.
    #[test]
    fn clamps_gain() -> Result<()> {
        let output = run(
            &[
                Complex::new(0.0, 0.0),
                Complex::new(0.0, 0.0),
                Complex::new(1.0, 0.0),
            ],
            Agc::builder(1.0, 1.0).max_gain(1.5),
        )?;
        assert_eq!(output[2], Complex::new(1.5, 0.0));

        let output = run(
            &[Complex::new(10.0, 0.0), Complex::new(1.0, 0.0)],
            Agc::builder(1.0, 1.0),
        )?;
        assert_eq!(output[1], Complex::new(0.0, 0.0));
        Ok(())
    }

    /// Verify synchronous tag propagation remains one-for-one.
    #[test]
    fn propagates_tags() -> Result<()> {
        let tag = Tag::new(1, "marker", TagValue::Bool(true));
        let (mut source, input) = VectorSource::builder(vec![Complex::new(1.0, 0.0); 2])
            .tags(std::slice::from_ref(&tag))
            .build()?;
        source.work()?;
        drop(source);

        let (mut agc, output) = Agc::new(input, 0.1, 0.01)?;
        agc.work()?;
        let (_, tags) = output.read_buf()?;
        assert!(tags.contains(&tag));
        Ok(())
    }

    /// Verify invalid public configurations fail before processing.
    #[test]
    fn validates_configuration() {
        for attack in [Float::NAN, Float::INFINITY, -0.1, 1.1] {
            assert!(
                Agc::new(ReadStream::from_slice(&[]), attack, 0.1).is_err(),
                "attack rate {attack} should be rejected"
            );
        }
        for decay in [Float::NAN, Float::INFINITY, -0.1, 1.1] {
            assert!(
                Agc::new(ReadStream::from_slice(&[]), 0.1, decay).is_err(),
                "decay rate {decay} should be rejected"
            );
        }
        assert!(
            Agc::builder(0.1, 0.01)
                .reference(0.0)
                .build(ReadStream::from_slice(&[]))
                .is_err()
        );
        assert!(
            Agc::builder(0.1, 0.01)
                .initial_gain(Float::NAN)
                .build(ReadStream::from_slice(&[]))
                .is_err()
        );
        assert!(
            Agc::builder(0.1, 0.01)
                .max_gain(0.0)
                .build(ReadStream::from_slice(&[]))
                .is_err()
        );
        assert!(
            Agc::builder(0.1, 0.01)
                .initial_gain(2.0)
                .max_gain(1.0)
                .build(ReadStream::from_slice(&[]))
                .is_err()
        );
    }
}
