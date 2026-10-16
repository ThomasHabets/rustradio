//! FIR kernels and blocks require only their actual arithmetic capabilities.
use rustradio::block::{Block, BlockRet};
use rustradio::fir::{Fir, FirFilter, FirFilterBuilder};
use rustradio::stream::{Tag, TagValue, new_stream};
use rustradio::{AlgebraicAdd, AlgebraicMul, Complex, Result};

// Neither type implements Default, Debug, Sample, or serialization.
#[derive(Clone, Copy)]
struct Number(f32);
#[derive(Clone, Copy)]
struct Coefficient(f32);

impl AlgebraicAdd for Number {
    type Output = Self;
    fn algebraic_add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}
impl AlgebraicMul<Coefficient> for Number {
    type Output = Self;
    fn algebraic_mul(self, rhs: Coefficient) -> Self {
        Self(self.0 * rhs.0)
    }
}

#[test]
fn slice_kernel_needs_neither_storage_nor_encoding_traits() -> Result<()> {
    let fir = Fir::new([Coefficient(2.0), Coefficient(3.0)])?;
    let input = [Number(5.0), Number(7.0), Number(11.0), Number(13.0)];
    assert_eq!(fir.filter(&input).0, 29.0);
    let out = fir.filter_n(&input, 2);
    assert_eq!(out.iter().map(|x| x.0).collect::<Vec<_>>(), [29.0, 59.0]);
    let mut out = [Number(-1.0); 2];
    fir.filter_n_inplace(&input, 2, &mut out);
    assert_eq!(out.map(|x| x.0), [29.0, 59.0]);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stored(f32);

impl Default for Stored {
    fn default() -> Self {
        Self(10.0)
    }
}
impl AlgebraicAdd for Stored {
    type Output = Self;
    fn algebraic_add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}
impl AlgebraicMul<Coefficient> for Stored {
    type Output = Self;
    fn algebraic_mul(self, rhs: Coefficient) -> Self {
        Self(self.0 * rhs.0)
    }
}

#[test]
fn streaming_custom_samples_need_no_encoding_or_translation_hooks() -> Result<()> {
    let (writer, input) = new_stream()?;
    let (mut filter, output) = FirFilter::builder([Coefficient(1.0)])
        .deci(2)
        .build(input)?;
    let mut window = writer.write_buf()?;
    window.fill_from_slice(&[Stored(2.0), Stored(3.0), Stored(5.0), Stored(7.0)]);
    window.produce(4, &[Tag::new(2, "mark", TagValue::Bool(true))]);
    drop(writer);
    assert!(matches!(filter.work()?, BlockRet::Again));
    assert!(matches!(filter.work()?, BlockRet::WaitForStream(_, 2)));
    let (window, tags) = output.read_buf()?;
    assert_eq!(window.slice(), &[Stored(2.0), Stored(5.0)]);
    assert_eq!(tags, [Tag::new(1, "mark", TagValue::Bool(true))]);
    Ok(())
}

#[test]
fn complex_streams_accept_real_coefficients() -> Result<()> {
    let (writer, input) = new_stream()?;
    let (mut filter, output): (FirFilter<Complex, f32>, _) = FirFilter::new(input, [2.0, 3.0])?;
    let mut window = writer.write_buf()?;
    window.fill_from_slice(&[Complex::new(5.0, 1.0), Complex::new(7.0, 2.0)]);
    window.produce(2, &[]);
    drop(writer);
    filter.work()?;
    assert_eq!(output.read_buf()?.0.slice(), &[Complex::new(29.0, 7.0)]);
    Ok(())
}

#[test]
fn translated_real_coefficients_match_explicit_input_mixing() -> Result<()> {
    let taps = [2.0f32, 3.0];
    let input: Vec<_> = (0..21).map(|i| Complex::new(i as f32, 1.0)).collect();
    let mixed: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            // fs=8, frequency=2: one quarter turn per input sample.
            let phase = -(i as f32) * std::f32::consts::FRAC_PI_2;
            x * Complex::new(phase.cos(), phase.sin())
        })
        .collect();
    let expected = Fir::new(taps)?.filter_n(&mixed, 2);
    let (writer, input_stream) = new_stream()?;
    let (mut filter, output) = FirFilter::builder(taps)
        .translate(8.0, 2.0)
        .deci(2)
        .build(input_stream)?;
    // Split across work calls to verify that the translation phase survives.
    for batch in input.chunks(10) {
        let mut window = writer.write_buf()?;
        window.fill_from_slice(batch);
        window.produce(batch.len(), &[]);
        filter.work()?;
    }
    let (window, _) = output.read_buf()?;
    assert_eq!(window.len(), expected.len());
    for (&actual, expected) in window.iter().zip(expected) {
        assert!(
            (actual - expected).norm() < 1e-4,
            "{actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn translation_settings_preserve_builder_and_filter_types() -> Result<()> {
    let (writer, input) = new_stream()?;
    let builder: FirFilterBuilder<Complex, f32> = FirFilter::builder([1.0]);
    let builder: FirFilterBuilder<Complex, f32> = builder.translate(8.0, 2.0);
    let (mut filter, output): (FirFilter<Complex, f32>, _) = builder.build(input)?;
    filter.disable_translation()?;
    let mut window = writer.write_buf()?;
    window.fill_from_slice(&[Complex::new(2.0, 3.0)]);
    window.produce(1, &[]);
    filter.work()?;
    assert_eq!(output.read_buf()?.0.slice(), &[Complex::new(2.0, 3.0)]);
    Ok(())
}

fn exercise_retuning<C>(taps: [C; 3]) -> Result<()>
where
    C: Copy + Send + Into<Complex> + 'static,
    Complex: AlgebraicMul<C, Output = Complex>,
{
    let (writer, input) = new_stream()?;
    let (mut filter, output): (FirFilter<Complex, C>, _) =
        FirFilter::builder(taps).deci(2).build(input)?;
    let control = filter.control();
    // Exercise the handle after ownership of the block has been erased/moved.
    let mut filter: Box<dyn Block> = Box::new(filter);
    let complex_taps = taps.map(Into::into);
    let ordinary = Fir::new(taps)?;
    let mut pending = Vec::new();
    let mut phase = 0.0f64;
    let mut next_input = 0usize;
    for setting in [
        Some((32.0, 3.0)),
        Some((32.0, -5.0)),
        Some((64.0, 1.5)),
        Some((64.0, 0.0)),
        None,
        Some((32.0, 2.0)),
        Some((32.0, 3.0)),
        None,
    ] {
        let sender = control.clone();
        std::thread::spawn(move || {
            // An intermediate request must not affect phase or coefficients.
            sender.set_translation(32.0, 7.0)?;
            match setting {
                Some((rate, freq)) => sender.set_translation(rate, freq),
                None => sender.disable_translation(),
            }
        })
        .join()
        .unwrap()?;
        assert!(matches!(filter.work()?, BlockRet::WaitForStream(_, 4)));

        let batch: Vec<_> = (next_input..next_input + 8)
            .map(|i| Complex::new((i as f32 * 0.17).sin(), (i as f32 * 0.23).cos()))
            .collect();
        next_input += batch.len();
        pending.extend_from_slice(&batch);
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&batch);
        window.produce(batch.len(), &[Tag::new(2, "retune", TagValue::Bool(true))]);

        let complete_windows = pending.len() - taps.len() + 1;
        let count = complete_windows / 2;
        let consumed = count * 2;
        let step = if let Some((rate, freq)) = setting {
            -2.0 * std::f64::consts::PI * f64::from(freq) / f64::from(rate)
        } else {
            phase = 0.0;
            0.0
        };
        // Independent reference: explicitly mix every input in each window,
        // then apply the original taps, rather than rotating coefficients.
        let expected: Vec<_> = (0..count)
            .map(|j| {
                let mut sum = Complex::new(0.0, 0.0);
                for i in 0..taps.len() {
                    let angle = phase + step * (j * 2 + i) as f64;
                    let mixer = Complex::new(angle.cos() as f32, angle.sin() as f32);
                    sum += pending[j * 2 + i] * mixer * complex_taps[taps.len() - 1 - i];
                }
                sum
            })
            .collect();
        assert!(matches!(filter.work()?, BlockRet::Again));
        let (window, tags) = output.read_buf()?;
        assert_eq!(window.len(), count);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].key(), "retune");
        for (&actual, expected) in window.iter().zip(expected) {
            assert!(
                (actual - expected).norm() < 1e-5,
                "{actual:?} != {expected:?} ({setting:?})"
            );
        }
        if setting.is_none() {
            assert_eq!(window.slice(), &ordinary.filter_n(&pending, 2)[..count]);
        }
        window.consume(count);
        pending.drain(..consumed);
        phase = (phase + step * consumed as f64).rem_euclid(2.0 * std::f64::consts::PI);
    }
    drop(filter);
    assert!(control.set_translation(32.0, 1.0).is_err());
    assert!(control.disable_translation().is_err());
    Ok(())
}

#[test]
fn runtime_retuning_preserves_phase_and_original_real_coefficients() -> Result<()> {
    exercise_retuning([0.2f32, 0.3, 0.5])
}

#[test]
fn runtime_retuning_preserves_phase_and_original_complex_coefficients() -> Result<()> {
    exercise_retuning([
        Complex::new(0.2, 0.1),
        Complex::new(0.3, -0.2),
        Complex::new(0.5, 0.05),
    ])
}

#[test]
fn invalid_retunes_leave_valid_pending_settings_unchanged() -> Result<()> {
    let (writer, input) = new_stream()?;
    let (mut filter, output) = FirFilter::new(input, [1.0f32])?;
    filter.set_translation(8.0, 2.0)?;
    let control = filter.control();
    for rate in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(control.set_translation(rate, 1.0).is_err());
    }
    for freq in [f32::NAN, f32::INFINITY] {
        assert!(control.set_translation(8.0, freq).is_err());
    }
    let mut window = writer.write_buf()?;
    window.fill_from_slice(&[Complex::new(1.0, 0.0); 4]);
    window.produce(4, &[]);
    filter.work()?;
    let (window, _) = output.read_buf()?;
    for (&actual, expected) in window.iter().zip([
        Complex::new(1.0, 0.0),
        Complex::new(0.0, -1.0),
        Complex::new(-1.0, 0.0),
        Complex::new(0.0, 1.0),
    ]) {
        assert!((actual - expected).norm() < 1e-6);
    }
    Ok(())
}
