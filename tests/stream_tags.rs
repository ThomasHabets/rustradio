//! Tag ordering and lifetime must survive sample-buffer and tag-queue wrapping.
use rustradio::Result;
use rustradio::stream::{ReadStream, Tag, TagValue, WriteStream, new_stream};

fn tag(pos: usize, id: u64) -> Tag {
    Tag::new(pos, "marker", TagValue::U64(id))
}

fn produce(writer: &WriteStream<u32>, count: usize, tags: &[Tag]) -> Result<()> {
    let mut window = writer.write_buf()?;
    window.slice()[..count].fill(42);
    window.produce(count, tags);
    Ok(())
}

fn check(reader: &ReadStream<u32>, count: usize, expected: &[Tag], consume: usize) -> Result<()> {
    let (window, tags) = reader.read_buf()?;
    assert_eq!(window.len(), count);
    assert!(window.iter().all(|sample| *sample == 42));
    assert_eq!(tags, expected);
    window.consume(consume);
    Ok(())
}

#[test]
fn tags_survive_wraparound_partial_consumption_and_unsorted_batches() -> Result<()> {
    let (writer, reader) = new_stream();
    let capacity = reader.total_size();
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
    let (writer, reader) = new_stream();
    let capacity = reader.total_size();
    for _ in 0..3 {
        let tags = [tag(0, 1), tag(capacity - 1, 2), tag(capacity - 1, 3)];
        produce(&writer, capacity, &tags)?;
        check(&reader, capacity, &tags, capacity)?;
        check(&reader, 0, &[], 0)?;
    }
    Ok(())
}
