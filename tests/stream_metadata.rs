//! Endpoint metadata is shared without affecting sample transfer or ownership.
use rustradio::Result;
use rustradio::stream::{Tag, TagValue, new_nocopy_stream, new_stream};

#[test]
fn names_are_shared_and_duplicates_are_disambiguated() -> Result<()> {
    let (writer, reader) = new_stream::<u8>()?;
    let (messages, receiver) = new_nocopy_stream::<String>();
    let original = reader.name();
    assert!(original.starts_with("stream-"));
    assert_eq!(writer.name(), original);
    assert_ne!(receiver.name(), original);
    assert_eq!(messages.name(), receiver.name());
    let writer = writer.with_name("");
    assert_eq!(writer.name(), original);
    let filtered = format!("filtered-{original}");
    let output = format!("output-{original}");
    let reader = reader.with_name(&filtered);
    assert_eq!(writer.name(), filtered);
    let writer = writer.with_name(output.clone());
    assert_eq!(reader.name(), output);
    let reader = reader.with_name("");
    assert_eq!(reader.name(), output);
    assert_eq!(writer.name(), output);
    let messages = messages.with_name(&output);
    assert_ne!(messages.name(), output);
    assert!(messages.name().starts_with(&output));
    assert_eq!(receiver.name(), messages.name());
    let name = format!("messages-{original}");
    let _receiver = receiver.with_name(&name);
    assert_eq!(messages.name(), name);
    Ok(())
}

#[test]
fn sample_rates_are_optional_and_shared_by_all_endpoints() -> Result<()> {
    let (writer, reader) = new_stream::<f32>()?;
    assert_eq!(writer.sample_rate(), None);
    assert_eq!(reader.sample_rate(), None);
    let writer = writer.with_sample_rate(48_000f32)?;
    assert_eq!(reader.sample_rate(), Some(48_000.));
    let reader = reader.with_sample_rate(96_000f64)?;
    assert_eq!(writer.sample_rate(), Some(96_000.));
    let _writer = writer.with_sample_rate_hz(24_000.)?;
    assert_eq!(reader.sample_rate(), Some(24_000.));

    let (writer, reader) = new_nocopy_stream::<String>();
    assert_eq!(writer.sample_rate(), None);
    assert_eq!(reader.sample_rate(), None);
    let writer = writer.with_sample_rate(8u32)?;
    assert_eq!(reader.sample_rate(), Some(8.));
    let reader = reader.with_sample_rate(16f32)?;
    assert_eq!(writer.sample_rate(), Some(16.));
    let _reader = reader.with_sample_rate_hz(32.)?;
    assert_eq!(writer.sample_rate(), Some(32.));
    Ok(())
}

#[test]
fn invalid_rates_are_rejected_without_overwriting_metadata() -> Result<()> {
    for rate in [0., -1., f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let (writer, reader) = new_stream::<u8>()?;
        let writer = writer.with_sample_rate(48_000.)?;
        assert!(reader.with_sample_rate(rate).is_err());
        assert_eq!(writer.sample_rate(), Some(48_000.));
        let (writer, reader) = new_nocopy_stream::<String>();
        let reader = reader.with_sample_rate(8.)?;
        assert!(writer.with_sample_rate(rate).is_err());
        assert_eq!(reader.sample_rate(), Some(8.));
    }
    Ok(())
}

#[test]
fn metadata_preserves_data_tags_and_eof() -> Result<()> {
    let (writer, reader) = new_stream::<u8>()?;
    let writer = writer.with_name("samples").with_sample_rate(2.)?;
    let mut buffer = writer.write_buf()?;
    buffer.fill_from_slice(&[7, 8]);
    let tags = vec![Tag::new(1, "test", TagValue::Bool(true))];
    buffer.produce(2, tags.clone());
    drop(writer);
    assert!(!reader.eof());
    assert_eq!(reader.name(), "samples");
    let (buffer, received_tags) = reader.read_buf()?;
    assert_eq!(buffer.slice(), &[7, 8]);
    assert_eq!(received_tags, tags);
    buffer.consume(2);
    assert!(reader.eof());

    let value = String::from("borrowed");
    let (writer, reader) = new_nocopy_stream();
    let writer = writer.with_name("messages").with_sample_rate(1.)?;
    writer.push(value, []);
    drop(writer);
    assert_eq!(reader.name(), "messages");
    assert_eq!(reader.pop().unwrap().0, "borrowed");
    assert!(reader.eof());
    Ok(())
}

#[test]
fn blockchain_arguments_accept_metadata() -> Result<()> {
    use rustradio::blocks::{AddConst, ConstantSource};
    use rustradio::graph::{Graph, GraphRunner};
    let (block, prev) = ConstantSource::new(1f32)?;
    let mut graph = Graph::new();
    graph.add(Box::new(block));
    let output = rustradio::blockchain![
        graph,
        prev,
        AddConst::new(prev.with_name("input").with_sample_rate(48_000.)?, 2.)?,
    ];
    assert_eq!(output.sample_rate(), None);
    let name = format!("filtered-{}", output.name());
    let output = output.with_name(&name).with_sample_rate(48_000.)?;
    assert_eq!(output.name(), name);
    assert_eq!(output.sample_rate(), Some(48_000.));
    Ok(())
}
