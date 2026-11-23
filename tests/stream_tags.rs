//! Tag ordering and lifetime must survive sample-buffer and tag-queue wrapping.
use std::borrow::Cow;

use rustradio::Result;
use rustradio::stream::{ReadStream, Tag, TagValue, WriteStream, new_stream};

fn tag(pos: usize, id: u64) -> Tag {
    Tag::new(pos, "marker", TagValue::U64(id))
}

fn produce<'a>(
    writer: &WriteStream<u32>,
    count: usize,
    tags: impl Into<Cow<'a, [Tag]>>,
) -> Result<()> {
    let mut window = writer.write_buf()?;
    window.slice()[..count].fill(42);
    window.produce(count, tags);
    Ok(())
}

fn check(reader: &ReadStream<u32>, count: usize, expected: &[Tag], consume: usize) -> Result<()> {
    let window = reader.read_buf()?;
    let tags = window.tags().cloned().collect::<Vec<_>>();
    assert_eq!(window.len(), count);
    assert!(window.iter().all(|sample| *sample == 42));
    assert_eq!(tags, expected);
    window.consume_discard_tags(consume);
    Ok(())
}

#[test]
fn tags_survive_wraparound_partial_consumption_and_unsorted_batches() -> Result<()> {
    let (writer, reader) = new_stream()?;
    let capacity = reader.capacity();
    for _ in 0..4 {
        // Leave both sample cursors close to the end of the physical buffer.
        produce(&writer, capacity - 4, &[tag(capacity - 5, 0)])?;
        check(&reader, capacity - 4, &[tag(capacity - 5, 0)], capacity - 4)?;
        produce(&writer, 8, &[tag(7, 4), tag(4, 2), tag(0, 1), tag(4, 3)])?;
        let expected = [tag(0, 1), tag(4, 2), tag(4, 3), tag(7, 4)];
        // Merely obtaining a read window must not consume tags.
        check(&reader, 8, &expected, 0)?;
        check(&reader, 8, &expected, 4)?;
        check(&reader, 4, &[tag(0, 2), tag(0, 3), tag(3, 4)], 1)?;
        // Append another unsorted batch while older tags remain queued.
        produce(&writer, 4, &[tag(3, 7), tag(0, 5), tag(0, 6)])?;
        check(&reader, 7, &[tag(2, 4), tag(3, 5), tag(3, 6), tag(6, 7)], 6)?;
        check(&reader, 1, &[tag(0, 7)], 1)?;
        check(&reader, 0, &[], 0)?;
        // Advance to the next physical buffer boundary.
        produce(&writer, capacity - 8, &[])?;
        check(&reader, capacity - 8, &[], capacity - 8)?;
    }
    Ok(())
}

#[test]
fn full_buffer_consumption_removes_all_tags() -> Result<()> {
    let (writer, reader) = new_stream()?;
    let capacity = reader.capacity();
    for _ in 0..3 {
        let tags = [tag(0, 1), tag(capacity - 1, 2), tag(capacity - 1, 3)];
        produce(&writer, capacity, &tags)?;
        check(&reader, capacity, &tags, capacity)?;
        check(&reader, 0, &[], 0)?;
    }
    Ok(())
}

#[test]
fn owned_tags_mix_with_borrowed_batches_across_wraparound() -> Result<()> {
    let (writer, reader) = new_stream()?;
    let capacity = reader.capacity();
    produce(&writer, capacity - 4, &[])?;
    check(&reader, capacity - 4, &[], capacity - 4)?;

    produce(&writer, 0, vec![])?;
    check(&reader, 0, &[], 0)?;
    produce(&writer, 8, vec![tag(7, 4), tag(4, 2), tag(0, 1), tag(4, 3)])?;
    let expected = [tag(0, 1), tag(4, 2), tag(4, 3), tag(7, 4)];
    check(&reader, 8, &expected, 0)?;
    check(&reader, 8, &expected, 4)?;
    // New borrowed tags must follow the retained owned tags.
    produce(&writer, 4, &[tag(3, 7), tag(0, 5), tag(0, 6)])?;
    check(
        &reader,
        8,
        &[
            tag(0, 2),
            tag(0, 3),
            tag(3, 4),
            tag(4, 5),
            tag(4, 6),
            tag(7, 7),
        ],
        5,
    )?;
    // Append another owned batch while a borrowed tag remains queued.
    produce(&writer, 4, vec![tag(3, 9), tag(0, 8)])?;
    check(&reader, 7, &[tag(2, 7), tag(3, 8), tag(6, 9)], 7)?;
    check(&reader, 0, &[], 0)?;
    Ok(())
}

#[test]
fn owned_production_rejects_an_oversized_write_and_releases_the_window() -> Result<()> {
    let (writer, reader) = new_stream::<u32>()?;
    let window = writer.write_buf()?;
    let count = window.len() + 1;
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            window.produce(count, vec![]);
        }))
        .is_err()
    );
    produce(&writer, 1, vec![tag(0, 1)])?;
    check(&reader, 1, &[tag(0, 1)], 1)?;
    Ok(())
}

#[test]
#[should_panic(expected = "tag outside produced samples")]
fn owned_production_rejects_an_out_of_range_tag() {
    let (writer, _reader) = new_stream::<u32>().unwrap();
    let mut window = writer.write_buf().unwrap();
    window.fill_from_slice(&[42]);
    window.produce(1, vec![tag(1, 1)]);
}

#[test]
fn read_window_moves_tags_across_partial_reads_and_interleaved_writes() -> Result<()> {
    let (writer, reader) = new_stream::<u32>()?;
    let (output, forwarded) = new_stream::<u32>()?;
    let original = Tag::new(1, "owned key", TagValue::String("owned value".into()));
    let key_ptr = original.key().as_ptr();
    let value_ptr = match original.val() {
        TagValue::String(value) => value.as_ptr(),
        _ => unreachable!(),
    };
    produce(&writer, 4, vec![original, tag(2, 1), tag(2, 2)])?;

    let window = reader.read_buf()?;
    produce(&writer, 1, vec![tag(0, 3)])?;
    assert_eq!(window.tags().len(), 3);
    assert_eq!(window.tags().next().unwrap().key().as_ptr(), key_ptr);
    drop(window);
    assert!(reader.read_buf()?.consume(0).is_empty());

    let window = reader.read_buf()?;
    assert_eq!(
        window.tags().map(|t| t.pos()).collect::<Vec<_>>(),
        [1, 2, 2, 4]
    );
    let mut out = output.write_buf()?;
    out.fill_from_slice(&window.slice()[..2]);
    out.produce(2, window.consume(2));
    let window = forwarded.read_buf()?;
    let samples = window.slice();
    let moved: &Tag = window.tags().next().unwrap();
    assert_eq!(samples[moved.pos()], 42);
    assert_eq!(moved.pos(), 1);
    assert_eq!(moved.key().as_ptr(), key_ptr);
    assert!(matches!(moved.val(), TagValue::String(s) if s.as_ptr() == value_ptr));

    let window = reader.read_buf()?;
    produce(&writer, 1, vec![Tag::new(0, "last", TagValue::Bool(true))])?;
    assert_eq!(window.consume(3), [tag(0, 1), tag(0, 2), tag(2, 3)]);
    let window = reader.read_buf()?;
    assert_eq!(window.tags().next().unwrap().pos(), 0);
    window.consume_discard_tags(1);
    assert!(reader.read_buf()?.is_empty());
    Ok(())
}
