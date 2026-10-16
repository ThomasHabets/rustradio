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
