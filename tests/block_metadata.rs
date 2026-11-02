//! Construction-time output metadata, including external derive expansion.
use rustradio::blocks::*;
use rustradio::stream::{StreamWait, new_nocopy_stream, new_stream, streams};
use rustradio::{Float, Result};

#[test]
fn names_rates_overrides_and_snapshot() -> Result<()> {
    let (writer, input) = new_stream::<Float>()?;
    let input = input.with_sample_rate(48_000.)?;
    let (_tee, left, right) = Tee::new(input)?;
    assert!(left.name().starts_with("auto/Tee/dst1"));
    assert!(right.name().starts_with("auto/Tee/dst2"));
    assert_ne!(left.name(), right.name());
    let entry = streams().into_iter().find(|s| s.id == left.id()).unwrap();
    assert!(entry.name.is_none());
    assert_eq!(entry.auto_name.as_deref(), Some(left.name().as_str()));
    assert_eq!(left.sample_rate(), Some(48_000.));
    let _writer = writer.with_sample_rate(96_000.)?;
    assert_eq!(left.sample_rate(), Some(48_000.));
    let left = left
        .with_name("explicit-metadata-test")
        .with_sample_rate(123.)?;
    assert_eq!(left.name(), "explicit-metadata-test");
    assert_eq!(left.sample_rate(), Some(123.));
    let (_map, mapped) = Map::keep_tags(right, "custom-transform", |x| x)?;
    assert!(mapped.name().starts_with("auto/custom-transform/dst"));
    assert_eq!(mapped.sample_rate(), Some(48_000.));
    Ok(())
}

#[test]
fn multiple_inputs_and_unknown_rates() -> Result<()> {
    for (a, b, expected) in [
        (Some(10.), Some(20.), None),
        (None, Some(20.), Some(20.)),
        (Some(20.), Some(20.), Some(20.)),
        (None, None, None),
    ] {
        let (_a, a_in) = new_stream::<Float>()?;
        let (_b, b_in) = new_stream::<Float>()?;
        let a_in = if let Some(rate) = a {
            a_in.with_sample_rate(rate)?
        } else {
            a_in
        };
        let b_in = if let Some(rate) = b {
            b_in.with_sample_rate(rate)?
        } else {
            b_in
        };
        let (_add, out) = Add::new(a_in, b_in)?;
        assert_eq!(out.sample_rate(), expected);
    }
    Ok(())
}

#[test]
fn fixed_ratios_and_nominal_clocks() -> Result<()> {
    let (_source, input) = SignalSourceFloat::new(48_000., 100., 1.)?;
    let (_fir, input) = FirFilter::builder(vec![1.0]).deci(4).build(input)?;
    assert_eq!(input.sample_rate(), Some(12_000.));
    let (_resampler, input) = RationalResampler::new(input, 2, 3)?;
    assert_eq!(input.sample_rate(), Some(8_000.));
    let (mut clock, input) = ZeroCrossing::new(input, 4., 0.)?;
    assert_eq!(input.sample_rate(), Some(2_000.));
    assert_eq!(clock.out_clock()?.sample_rate(), Some(2_000.));
    let (_chunks, messages) = StreamChunks::new(input, 10)?;
    assert_eq!(messages.sample_rate(), Some(200.));
    let (_average, messages) = PduAverage::new(messages, 5);
    assert_eq!(messages.sample_rate(), Some(40.));
    let (_flatten, samples) = PduToStream::new(messages)?;
    assert_eq!(samples.sample_rate(), None);
    let (_tx, bytes) = new_stream::<u8>()?;
    let (_parse, floats) = Parse::<Float>::new(bytes.with_sample_rate(192_000.)?)?;
    assert_eq!(floats.sample_rate(), Some(48_000.));
    Ok(())
}

#[test]
fn source_and_message_rates() -> Result<()> {
    let (_tx, rx) = new_nocopy_stream::<Vec<u8>>();
    let (_framer, out) = HdlcFramer::new(rx.with_sample_rate(2.)?);
    assert_eq!(out.sample_rate(), Some(2.));
    let (_tx, rx) = new_nocopy_stream::<Vec<u8>>();
    let (_decoder, out) = KissDecode::new(rx.with_sample_rate(2.)?);
    assert_eq!(out.sample_rate(), None);
    let (_constant, out) = ConstantSource::new(1u8)?;
    assert_eq!(out.sample_rate(), None);
    assert!(out.name().starts_with("auto/ConstantSource"));
    Ok(())
}

#[cfg(feature = "unstable")]
#[test]
fn automatic_names_do_not_enable_named_discovery() -> Result<()> {
    let (_block, out) = ConstantSource::new(1u8)?;
    let server = rustradio::iq_stream::IqServer::new();
    server.register_named_streams();
    assert!(!server.streams().iter().any(|s| s.name == out.name()));
    let name = format!("explicit-discovery-{}", out.id());
    let out = out.with_name(name);
    assert!(server.streams().iter().any(|s| s.name == out.name()));
    Ok(())
}

#[derive(rustradio::rustradio_macros::Block)]
#[rustradio(new)]
struct ScaledMetadata {
    #[rustradio(in)]
    src: rustradio::stream::ReadStream<Float>,
    #[rustradio(out, sample_rate = "self.src.sample_rate().map(|r| r * self.factor)")]
    dst: rustradio::stream::WriteStream<Float>,
    factor: f64,
}

#[test]
fn external_rate_attributes_and_invalid_inference() -> Result<()> {
    for (factor, expected) in [
        (2., Some(20.)),
        (0., None),
        (-1., None),
        (f64::MAX, None),
        (f64::NAN, None),
    ] {
        let (_tx, input) = new_stream::<Float>()?;
        let (_block, output) = ScaledMetadata::new(input.with_sample_rate(10.)?, factor)?;
        assert_eq!(output.sample_rate(), expected);
        assert_eq!(output.name(), "auto/ScaledMetadata/dst");
    }
    let (_a, first) = ConstantSource::new(1u8)?;
    let (_b, second) = ConstantSource::new(2u8)?;
    assert_ne!(first.name(), second.name());
    let second = second.with_name(first.name());
    assert_ne!(first.name(), second.name());
    Ok(())
}

#[test]
fn optional_output_initialization_preserves_existing_overrides() -> Result<()> {
    let (_tx, input) = new_stream::<Float>()?;
    let (mut block, output) = ZeroCrossing::new(input.with_sample_rate(48_000.)?, 4., 0.)?;
    let name = format!("explicit-clock-output-{}", output.id());
    let output = output.with_name(&name).with_sample_rate(123.)?;
    let clock = block.out_clock()?;
    assert_eq!(clock.sample_rate(), Some(12_000.));
    assert!(clock.name().starts_with("auto/ZeroCrossing/out_clock"));
    assert_eq!(output.sample_rate(), Some(123.));
    assert_eq!(output.name(), name);
    Ok(())
}

#[test]
fn wrapper_initializes_internal_streams() -> Result<()> {
    let (_tx, input) = new_stream::<Float>()?;
    let (_block, output) = FftFilterFloat::new(input.with_sample_rate(48_000.)?, &[1.0])?;
    assert_eq!(output.sample_rate(), Some(48_000.));
    let internal = streams()
        .into_iter()
        .filter(|s| {
            s.name().starts_with("auto/FftFilterFloat/inner_in")
                || s.name().starts_with("auto/FftFilter/dst")
        })
        .collect::<Vec<_>>();
    assert_eq!(internal.len(), 2);
    assert!(internal.iter().all(|s| s.sample_rate_hz == Some(48_000.)));
    Ok(())
}
