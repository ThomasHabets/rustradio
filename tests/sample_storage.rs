//! Processing samples does not require defining a byte encoding.
use rustradio::Result;
use rustradio::block::Block;
use rustradio::blocks::{Head, Map, VectorSink, VectorSource};

#[derive(Debug, Copy, Clone, Default, PartialEq)]
struct InMemory {
    value: u32,
    valid: bool,
}

#[test]
fn custom_sample_without_encoding_flows_through_processing_blocks() -> Result<()> {
    let input = vec![
        InMemory {
            value: 3,
            valid: true,
        },
        InMemory {
            value: 5,
            valid: false,
        },
        InMemory {
            value: 7,
            valid: true,
        },
    ];
    let (mut source, stream) = VectorSource::new(input)?;
    let (mut map, stream) = Map::keep_tags(stream, "double", |mut sample: InMemory| {
        sample.value *= 2;
        sample
    })?;
    let (mut head, stream) = Head::new(stream, 2)?;
    let mut sink = VectorSink::new(stream, 100);
    let hook = sink.hook();
    source.work()?;
    map.work()?;
    head.work()?;
    sink.work()?;
    assert_eq!(
        hook.data().samples(),
        &[
            InMemory {
                value: 6,
                valid: true
            },
            InMemory {
                value: 10,
                valid: false
            },
        ]
    );
    Ok(())
}

#[test]
fn iterator_fill_reports_count_and_rejects_overflow() -> Result<()> {
    let (output, input) = rustradio::stream::new_stream::<u8>()?;
    let mut window = output.write_buf()?;
    let capacity = window.len();
    let mut oversized = std::iter::repeat_n(7, capacity + 2);
    assert!(window.fill_from_iter(&mut oversized).is_err());
    assert_eq!(oversized.len(), 1);
    drop(window);
    assert!(input.read_buf()?.is_empty());

    let mut window = output.write_buf()?;
    assert_eq!(window.fill_from_iter([])?, 0);
    assert_eq!(window.fill_from_iter([3, 4])?, 2);
    let written = window.fill_from_iter(std::iter::repeat_n(5, capacity))?;
    assert_eq!(written, capacity);
    window.produce(written, &[]);
    assert!(input.read_buf()?.iter().all(|sample| *sample == 5));
    Ok(())
}
