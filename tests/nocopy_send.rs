//! Owned messages need Send, but do not need Sync.
use std::cell::Cell;
use std::time::Duration;

use rustradio::Result;
use rustradio::block::{Block, BlockRet};
use rustradio::blocks::{NCMap, Strobe, VectorSinkNoCopy};
use rustradio::stream::StreamWait;

#[test]
fn send_only_messages_pass_through_blocks() -> Result<()> {
    let (mut source, input) = Strobe::new(Duration::MAX, Cell::new(3u8));
    assert!(matches!(source.work()?, BlockRet::Pending));
    assert!(!input.closed());
    let (mut map, output) = NCMap::new(input, "cell", |value: Cell<u8>, tags| {
        value.set(value.get() + 1);
        vec![(value, tags)]
    });
    map.work()?;
    let mut sink = VectorSinkNoCopy::new(output, 1);
    sink.work()?;
    assert_eq!(sink.storage().lock().unwrap()[0].0.get(), 4);
    Ok(())
}
