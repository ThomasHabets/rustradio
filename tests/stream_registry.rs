//! Process-wide discovery must not retain stream storage or change primary EOF.
use rustradio::Result;
use rustradio::stream::{StreamInfo, StreamWait, new_nocopy_stream, new_stream, streams};

fn info(name: &str) -> StreamInfo {
    streams().into_iter().find(|s| s.name() == name).unwrap()
}

#[test]
fn inventory_tracks_metadata_updates_and_completion() -> Result<()> {
    let (writer, reader) = new_stream::<f32>()?;
    let name = reader.name();
    let original = info(&name);
    assert!(original.name.is_none());
    assert_eq!(original.id, writer.id());
    assert_eq!(original.name(), format!("stream-{}", original.id));
    let json = serde_json::to_value(&original)?;
    assert_eq!(json["id"], original.id);
    assert!(json["name"].is_null());
    assert!(json.get("named").is_none());
    assert!(!original.messages);
    assert!(!original.ended);
    assert_eq!(original.sample_rate_hz, None);
    assert_eq!(original.sample_type, std::any::type_name::<f32>());

    let writer = writer.with_name("");
    assert!(info(&name).name.is_none());
    let writer = writer.with_name(&name).with_sample_rate(48_000.)?;
    assert_eq!(info(&name).name.as_deref(), Some(name.as_str()));
    assert_eq!(info(&name).sample_rate_hz, Some(48_000.));
    let reader = reader.with_sample_rate(96_000.)?;
    assert_eq!(info(&name).sample_rate_hz, Some(96_000.));
    let renamed = format!("inventory-{name}");
    let writer = writer.with_name(&renamed);
    assert!(!streams().iter().any(|s| s.name() == name));
    assert_eq!(reader.name(), renamed);
    let writer = writer.with_name("");
    assert_eq!(info(&renamed).name.as_deref(), Some(renamed.as_str()));
    assert_eq!(info(&renamed).id, original.id);
    drop(writer);
    assert!(info(&renamed).ended);
    assert!(reader.eof());
    drop(reader);
    assert!(!streams().iter().any(|s| s.name() == renamed));
    // Snapshots are owned values and remain usable after the stream expires.
    assert_eq!(original.name(), name);
    Ok(())
}

#[test]
fn inventory_lifetime_follows_outstanding_windows() -> Result<()> {
    let (writer, reader) = new_stream::<u8>()?;
    let name = reader.name();
    let mut writing = writer.write_buf()?;
    writing.fill_from_slice(&[7]);
    drop(writer);
    assert!(!info(&name).ended);
    writing.produce(1, &[]);
    assert!(info(&name).ended);
    assert!(!reader.eof());
    let (reading, _) = reader.read_buf()?;
    assert_eq!(reading.slice(), &[7]);
    drop(reader);
    assert!(info(&name).ended);
    drop(reading);
    assert!(!streams().iter().any(|s| s.name() == name));
    Ok(())
}

#[test]
fn inventory_includes_custom_message_types_without_retaining_them() {
    use std::rc::Rc;
    let value = Rc::new(String::from("custom"));
    let (writer, reader) = new_nocopy_stream::<Rc<String>>();
    let name = reader.name();
    let entry = info(&name);
    assert!(entry.messages);
    assert_eq!(entry.sample_type, std::any::type_name::<Rc<String>>());
    writer.push(value.clone(), []);
    assert_eq!(Rc::strong_count(&value), 2);
    drop(writer);
    assert!(info(&name).ended);
    assert!(!reader.eof());
    drop(reader);
    assert_eq!(Rc::strong_count(&value), 1);
    assert!(!streams().iter().any(|s| s.name() == name));
}

#[test]
fn concurrent_names_remain_unique_and_inventory_is_sorted() -> Result<()> {
    let (writer, reader) = new_stream::<u8>()?;
    let name = format!("concurrent-{}", reader.name());
    let _writer = writer.with_name(&name);
    let handles = std::thread::scope(|scope| {
        (0..8)
            .map(|_| {
                let name = &name;
                scope.spawn(move || {
                    let (writer, reader) = new_stream::<u8>().unwrap();
                    (writer.with_name(name), reader)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    let names: std::collections::HashSet<_> = handles.iter().map(|(_, r)| r.name()).collect();
    assert_eq!(names.len(), handles.len());
    assert!(!names.contains(&name));
    let inventory = streams();
    assert!(
        inventory
            .windows(2)
            .all(|pair| pair[0].name() < pair[1].name())
    );
    for (writer, reader) in &handles {
        assert_eq!(writer.name(), reader.name());
        assert!(reader.name().starts_with(&name));
        let entry = inventory
            .iter()
            .find(|s| s.name() == reader.name())
            .unwrap();
        assert!(entry.name.is_some());
    }
    Ok(())
}
