//! Stream ownership must preserve the unwind-safety guarantees of graph blocks.
use std::cell::Cell;
use std::panic::{RefUnwindSafe, UnwindSafe};

use rustradio::blocks::{
    NullSink, PduToStream, SignalSourceComplex, StreamToPdu, Tee, VectorSource,
};
use rustradio::stream::{NCReadStream, NCWriteStream, ReadStream, WriteStream};

fn assert_unwind_safe<T: UnwindSafe + RefUnwindSafe>() {}

#[test]
fn stream_endpoints_are_unwind_safe() {
    assert_unwind_safe::<ReadStream<f32>>();
    assert_unwind_safe::<WriteStream<f32>>();
    assert_unwind_safe::<NCReadStream<String>>();
    assert_unwind_safe::<NCWriteStream<String>>();
}

#[test]
fn nocopy_payloads_need_not_be_ref_unwind_safe() {
    // The stream's mutex protects the payload, including interior mutability.
    assert_unwind_safe::<NCReadStream<Cell<u8>>>();
    assert_unwind_safe::<NCWriteStream<Cell<u8>>>();
}

#[test]
fn blocks_with_copying_nocopy_and_mixed_ports_are_unwind_safe() {
    assert_unwind_safe::<SignalSourceComplex>();
    assert_unwind_safe::<VectorSource<f32>>();
    assert_unwind_safe::<NullSink<f32>>();
    assert_unwind_safe::<Tee<f32>>();
    assert_unwind_safe::<StreamToPdu<f32>>();
    assert_unwind_safe::<PduToStream<f32>>();
}
