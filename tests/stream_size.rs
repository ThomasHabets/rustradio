//! Check that a configured stream size reaches new streams and frame checks.
//!
//! Its own test binary, because the size is process wide.
use rustradio::Complex;
use rustradio::blocks::FftStream;
use rustradio::stream::{new_stream, set_stream_size, stream_size};

#[test]
fn configured_size_applies_to_new_streams() -> rustradio::Result<()> {
    const BYTES: usize = 1 << 20;
    set_stream_size(BYTES)?;
    assert_eq!(stream_size(), BYTES);

    let (_w, r) = new_stream::<Complex>()?;
    assert_eq!(r.total_size(), BYTES / std::mem::size_of::<Complex>());
    let (_w, r) = new_stream::<f32>()?;
    assert_eq!(r.total_size(), BYTES / std::mem::size_of::<f32>());

    // A frame that fits the stream is accepted; one that does not is rejected
    // at construction instead of waiting for space that never comes.
    let frames = BYTES / std::mem::size_of::<Complex>();
    let (_w, src) = new_stream::<Complex>()?;
    assert!(FftStream::new(src, frames).is_ok());
    let (_w, src) = new_stream::<Complex>()?;
    assert!(FftStream::new(src, frames * 2).is_err());
    Ok(())
}
