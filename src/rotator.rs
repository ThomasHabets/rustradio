//! Frequency translation through complex exponential.

use crate::block::{Block, BlockRet};
use crate::oscillator::ComplexOscillator;
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float, Result};

/// Frequency translate through complex exponential.
///
/// Rotates the frequency spectrum without filtering. Frequencies alias modulo
/// the sample rate. Positive frequency shifts upward. The first input is multiplied
/// by one, and subsequent inputs by `exp(i * n * 2π * freq / samp_rate)`.
/// At zero frequency the default rotation is the identity. Tags are preserved.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct Rotator {
    translator: ComplexOscillator,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl Rotator {
    /// Create a rotator from sample rate and frequency in Hz.
    ///
    /// Rejects nonfinite values and nonpositive sample rates before allocating output.
    pub fn new(
        src: ReadStream<Complex>,
        samp_rate: Float,
        freq: Float,
    ) -> Result<(Self, ReadStream<Complex>)> {
        let xlat = ComplexOscillator::new(samp_rate, freq)?;
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

    /// Change frequency in Hz, preserving the next sample's mixing phase.
    pub fn set_frequency(&mut self, samp_rate: Float, freq: Float) -> Result<()> {
        self.translator.set_frequency(samp_rate, freq)
    }

    /// Set the next sample's mixing phase in radians.
    pub fn set_phase(&mut self, phase: Float) -> Result<()> {
        self.translator.set_phase(phase)
    }
}

impl Block for Rotator {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            let (input, tags) = self.src.read_buf()?;
            if input.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            let mut output = self.dst.write_buf()?;
            if output.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let n = input.len().min(output.len());
            self.translator
                .mix(&input.slice()[..n], &mut output.slice()[..n])?;
            let tags: Vec<_> = tags.into_iter().take_while(|tag| tag.pos() < n).collect();
            output.produce(n, tags);
            input.consume(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockRet};
    use crate::stream::{Tag, TagValue, new_stream};

    #[test]
    fn zero_frequency_is_identity_and_preserves_tags() -> Result<()> {
        let samples = [Complex::new(2.0, 3.0), Complex::new(-4.0, 5.0)];
        let tag = Tag::new(1, "marker", TagValue::Bool(true));
        let (writer, input) = new_stream()?;
        let (mut rotator, output) = Rotator::new(input, 8.0, 0.0)?;
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&samples);
        window.produce(samples.len(), std::slice::from_ref(&tag));
        drop(writer);
        rotator.work()?;
        let (window, tags) = output.read_buf()?;
        assert_eq!(window.slice(), &samples);
        assert_eq!(tags, [tag]);
        Ok(())
    }

    #[test]
    fn signed_rotation_continues_across_batches_and_empty_work_calls() -> Result<()> {
        for frequency in [2.0, -2.0] {
            let (writer, input) = new_stream()?;
            let (mut rotator, output) = Rotator::new(input, 8.0, frequency)?;
            for batch in 0..3 {
                assert!(matches!(rotator.work()?, BlockRet::WaitForStream(_, _)));
                let mut window = writer.write_buf()?;
                window.slice()[..3].fill(Complex::new(1.0, 0.0));
                window.produce(3, &[]);
                rotator.work()?;
                let (window, _) = output.read_buf()?;
                assert_eq!(window.len(), 3);
                for (i, &sample) in window.iter().enumerate() {
                    let phase =
                        (batch * 3 + i) as f64 * std::f64::consts::TAU * f64::from(frequency) / 8.0;
                    let expected = Complex::new(phase.cos() as f32, phase.sin() as f32);
                    assert!((sample - expected).norm() < 1e-6);
                }
                window.consume(3);
            }
        }
        Ok(())
    }

    #[test]
    fn output_backpressure_preserves_input_and_phase() -> Result<()> {
        let (writer, input) = new_stream()?;
        let (mut rotator, output) = Rotator::new(input, 8.0, 2.0)?;
        let mut full = rotator.dst.write_buf()?;
        let capacity = full.len();
        full.slice().fill(Complex::new(99.0, 0.0));
        full.produce(capacity, &[]);
        let mut window = writer.write_buf()?;
        window.slice()[..2].fill(Complex::new(1.0, 0.0));
        window.produce(2, &[]);
        rotator.work()?;
        output.read_buf()?.0.consume(capacity);
        rotator.work()?;
        let (window, _) = output.read_buf()?;
        assert_eq!(window.len(), 2);
        assert_eq!(window.slice()[0], Complex::new(1.0, 0.0));
        assert!((window.slice()[1] - Complex::new(0.0, 1.0)).norm() < 1e-6);
        Ok(())
    }

    #[test]
    fn partial_output_preserves_remaining_samples_and_tags() -> Result<()> {
        let (writer, input) = new_stream()?;
        let (mut rotator, output) = Rotator::new(input, 8.0, 1.0)?;
        let mut prefill = rotator.dst.write_buf()?;
        let padding = prefill.len() - 2;
        prefill.slice()[..padding].fill(Complex::default());
        prefill.produce(padding, &[]);
        let samples = [Complex::new(2.0, -3.0); 5];
        let tags = [
            Tag::new(1, "first", TagValue::Bool(true)),
            Tag::new(3, "second", TagValue::Bool(true)),
        ];
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&samples);
        window.produce(5, &tags);
        drop(writer);
        let mut expected = ComplexOscillator::new(8.0, 1.0)?;
        assert!(matches!(rotator.work()?, BlockRet::WaitForStream(_, 1)));
        output.read_buf()?.0.consume(padding);
        for (count, name) in [(2, "first"), (3, "second")] {
            let (window, tags) = output.read_buf()?;
            assert_eq!(window.len(), count);
            for sample in window.iter() {
                assert_eq!(*sample, samples[0] * expected.next().unwrap());
            }
            assert_eq!(tags, [Tag::new(1, name, TagValue::Bool(true))]);
            window.consume(count);
            rotator.work()?;
        }
        assert!(rotator.src.eof());
        assert_eq!(rotator.translator.next(), expected.next());
        Ok(())
    }

    #[test]
    fn frequency_and_phase_can_be_changed_while_holding_the_block() -> Result<()> {
        let (writer, input) = new_stream()?;
        let (mut rotator, output) = Rotator::new(input, 8.0, 2.0)?;
        let mix_one = |rotator: &mut Rotator, sample| -> Result<Complex> {
            let mut window = writer.write_buf()?;
            window.slice()[0] = sample;
            window.produce(1, &[]);
            rotator.work()?;
            let (window, _) = output.read_buf()?;
            let result = window.slice()[0];
            window.consume(1);
            Ok(result)
        };
        let one = Complex::new(1.0, 0.0);
        assert_eq!(mix_one(&mut rotator, one)?, one);
        rotator.set_frequency(8.0, -2.0)?;
        assert!((mix_one(&mut rotator, one)? - Complex::new(0.0, 1.0)).norm() < 1e-6);
        assert!((mix_one(&mut rotator, one)? - one).norm() < 1e-6);
        rotator.set_phase(std::f32::consts::PI)?;
        assert!((mix_one(&mut rotator, one)? - Complex::new(-1.0, 0.0)).norm() < 1e-6);
        Ok(())
    }

    #[test]
    fn rejects_invalid_configuration() -> Result<()> {
        for rate in [0.0, -1.0, Float::NAN, Float::INFINITY] {
            let (_writer, input) = new_stream()?;
            assert!(Rotator::new(input, rate, 1.0).is_err());
        }
        for freq in [Float::NAN, Float::INFINITY] {
            let (_writer, input) = new_stream()?;
            assert!(Rotator::new(input, 8.0, freq).is_err());
        }
        Ok(())
    }
}
