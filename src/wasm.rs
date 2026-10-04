//! This module contains wasm versions of various code.
//!
//! It must fail gracefully when used in a web worker.
use std::sync::Arc;
use std::sync::Mutex;

use wasm_bindgen::prelude::*;

use crate::stream::Tag;
use crate::stream_tags::StreamTags;
use crate::{Error, Result};

pub mod wasm_graph;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
    #[wasm_bindgen(js_namespace = performance)]
    fn now() -> f64;
}

impl From<Error> for JsValue {
    fn from(e: Error) -> Self {
        JsValue::from_str(&format!("RustRadio: {e}"))
    }
}

pub fn initialize_rustradio() {
    log(&format!(
        "Initializing RustRadio {} rustc version {} git version {}",
        env!("CARGO_PKG_VERSION"),
        env!("RUSTC_VERSION"),
        env!("GIT_VERSION")
    ));
}

#[must_use]
pub(crate) fn get_cpu_time() -> std::time::Duration {
    // This is not available in WASM.
    // We could try using `performance.now()`, but that's wallclock time.
    std::time::Duration::from_secs(0)
}

pub(crate) fn sleep(_d: std::time::Duration) {}

/// Fake std::time::Instant.
pub(crate) struct Instant {
    ts: f64,
}
impl Instant {
    pub(crate) fn now() -> Self {
        Self { ts: Self::now2() }
    }
    fn now2() -> f64 {
        web_sys::window()
            .and_then(|v| v.performance())
            .map(|v| v.now())
            .unwrap_or_default()
    }
    pub(crate) fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_millis((Self::now2() - self.ts) as u64)
    }
}

// WASM cannot double-map the ring into a linear slice. Readers and writers
// therefore own linear scratch buffers and copy across at most two ring slices.
// The single writer reuses a full-sized, initialized buffer; reader snapshots
// reuse capacity when returned. Scratch storage never aliases the sample ring.
#[derive(Debug)]
struct BufferState<T> {
    rpos: usize,
    wpos: usize,
    used: usize,
    // Only the range described by rpos/used contains produced samples.
    stream: Vec<T>,
    tags: StreamTags,
    write_cache: Option<Vec<T>>,
    read_cache: Vec<T>,

    // Extra accounting to ensure that we never read uninitialized content.
    #[cfg(debug_assertions)]
    initialized: Vec<bool>,
}

impl<T: Default> BufferState<T> {
    const _CHECK_NOT_ZERO: () = assert!(
        std::mem::size_of::<T>() != 0,
        "Zero sized stream members are not supported"
    );

    /// Size in bytes.
    fn new(byte_size: usize) -> Result<Self> {
        let member_size = std::mem::size_of::<T>();
        let size = byte_size / member_size;
        if !byte_size.is_multiple_of(member_size) {
            return Err(Error::msg(format!(
                "Buffer size ({byte_size}) must be multiple of element size ({member_size})"
            )));
        }
        let stream = std::iter::repeat_with(T::default).take(size).collect();
        Ok(Self {
            rpos: 0,
            wpos: 0,
            used: 0,
            stream,
            tags: StreamTags::default(),
            write_cache: Some(std::iter::repeat_with(T::default).take(size).collect()),
            read_cache: Vec::new(),

            #[cfg(debug_assertions)]
            initialized: vec![false; size],
        })
    }
}

impl<T> BufferState<T> {
    fn ranges(
        &self,
        start: usize,
        count: usize,
    ) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let first = count.min(self.capacity() - start);
        (start..start + first, 0..count - first)
    }

    fn consume(&mut self, count: usize) {
        assert!(
            count <= self.used,
            "trying to consume {count}, but only have {}",
            self.used
        );
        if count == 0 {
            return;
        }
        #[cfg(debug_assertions)]
        {
            let (first, second) = self.ranges(self.rpos, count);
            debug_assert!(self.initialized[first.clone()].iter().all(|value| *value));
            debug_assert!(self.initialized[second.clone()].iter().all(|value| *value));
            self.initialized[first].fill(false);
            self.initialized[second].fill(false);
        }
        self.tags.consume(count);
        self.rpos = (self.rpos + count) % self.capacity();
        self.used -= count;
    }

    fn recycle_reader(&mut self, mut stream: Vec<T>) {
        if stream.capacity() >= self.read_cache.capacity() {
            stream.clear();
            self.read_cache = stream;
        }
    }

    #[must_use]
    fn capacity(&self) -> usize {
        self.size()
    }
    #[must_use]
    fn free(&self) -> usize {
        self.size() - self.used
    }
    #[must_use]
    fn size(&self) -> usize {
        self.stream.len()
    }
}

#[derive(Debug)]
pub struct Buffer<T> {
    id: usize,
    state: Mutex<BufferState<T>>,
}
impl<T: Default> Buffer<T> {
    pub fn new(size: usize) -> Result<Self> {
        Ok(Self {
            id: crate::NEXT_STREAM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            state: Mutex::new(BufferState::new(size)?),
        })
    }
}
impl<T> Buffer<T> {
    pub fn id(&self) -> usize {
        self.id
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.state.lock().unwrap().used == 0
    }
    /// Available space to write, in bytes(?).
    pub(crate) fn free(&self) -> usize {
        self.state.lock().unwrap().free()
    }
    pub fn consume(&self, n: usize) {
        self.state.lock().unwrap().consume(n);
    }
    pub fn total_size(&self) -> usize {
        self.state.lock().unwrap().capacity()
    }
    pub fn wait_for_write(&self, _need: usize) -> usize {
        // TODO
        self.free()
    }
    pub fn wait_for_read(&self, _need: usize) -> usize {
        // TODO
        self.state.lock().unwrap().used
    }
    #[cfg(feature = "async")]
    pub async fn wait_for_write_async(&self, _need: usize) -> usize {
        // TODO
        self.wait_for_write(_need)
    }
    #[cfg(feature = "async")]
    pub async fn wait_for_read_async(&self, _need: usize) -> usize {
        // TODO
        self.wait_for_read(_need)
    }
    pub fn write_buf(self: Arc<Self>) -> Result<BufferWriter<T>> {
        let mut state = self.state.lock().unwrap();
        let len = state.free();
        let stream = state
            .write_cache
            .take()
            .ok_or_else(|| Error::msg("write_buf() called with an outstanding write buffer"))?;
        drop(state);
        Ok(BufferWriter {
            parent: self,
            len,
            prepared: 0,
            stream: Some(stream),
        })
    }
}

impl<T: Copy> BufferState<T> {
    fn produce(&mut self, samples: &[T], tags: &[Tag]) {
        if samples.is_empty() {
            debug_assert!(tags.is_empty());
            return;
        }
        assert!(
            samples.len() <= self.free(),
            "tried to produce {}, but only {} is free out of {}",
            samples.len(),
            self.free(),
            self.capacity()
        );
        let (first, second) = self.ranges(self.wpos, samples.len());
        let split = first.len();
        #[cfg(debug_assertions)]
        {
            debug_assert!(self.initialized[first.clone()].iter().all(|value| !*value));
            debug_assert!(self.initialized[second.clone()].iter().all(|value| !*value));
            self.initialized[first.clone()].fill(true);
            self.initialized[second.clone()].fill(true);
        }
        self.stream[first].copy_from_slice(&samples[..split]);
        self.stream[second].copy_from_slice(&samples[split..]);
        self.tags.produce(samples.len(), tags);
        self.wpos = (self.wpos + samples.len()) % self.capacity();
        self.used += samples.len();
    }
}

impl<T: Copy> Buffer<T> {
    pub fn produce(&self, samples: &[T], tags: &[Tag]) {
        self.state.lock().unwrap().produce(samples, tags);
    }

    pub fn read_buf(self: Arc<Self>) -> Result<(BufferReader<T>, Vec<Tag>)> {
        let mut state = self.state.lock().unwrap();
        let (first, second) = state.ranges(state.rpos, state.used);
        #[cfg(debug_assertions)]
        {
            debug_assert!(state.initialized[first.clone()].iter().all(|value| *value));
            debug_assert!(state.initialized[second.clone()].iter().all(|value| *value));
        }
        let mut stream = std::mem::take(&mut state.read_cache);
        stream.reserve(state.used);
        stream.extend_from_slice(&state.stream[first]);
        stream.extend_from_slice(&state.stream[second]);
        let tags = state.tags.read();
        drop(state);
        Ok((BufferReader::new(self, stream), tags))
    }
}

pub struct BufferReader<T> {
    parent: Arc<Buffer<T>>,
    stream: Vec<T>,
}
impl<T> BufferReader<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, stream: Vec<T>) -> Self {
        Self { parent, stream }
    }

    /// Return slice to read from.
    #[must_use]
    pub fn slice(&self) -> &[T] {
        &self.stream
    }

    /// Helper function to iterate over input instead.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.slice().iter()
    }

    /// We're done with the buffer. Consume `n` samples.
    pub fn consume(mut self, n: usize) {
        assert!(
            n <= self.stream.len(),
            "trying to consume {n}, but read buffer only has {}",
            self.stream.len()
        );
        let mut state = self.parent.state.lock().unwrap();
        state.consume(n);
        state.recycle_reader(std::mem::take(&mut self.stream));
    }

    /// len convenience function.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slice().len()
    }

    /// is_empty convenience function.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl<T> Drop for BufferReader<T> {
    fn drop(&mut self) {
        if self.stream.capacity() != 0
            && let Ok(mut state) = self.parent.state.lock()
        {
            state.recycle_reader(std::mem::take(&mut self.stream));
        }
    }
}

pub struct BufferWriter<T> {
    parent: Arc<Buffer<T>>,
    len: usize,
    prepared: usize,
    // None after the storage has been returned to the parent.
    stream: Option<Vec<T>>,
}

impl<T> BufferWriter<T> {
    /// Copy from an iterator, stopping at the end of the write window.
    pub fn fill_from_iter(&mut self, src: impl IntoIterator<Item = T>) {
        self.prepared = 0;
        let stream = self.stream.as_mut().expect("writer storage");
        for (place, item) in stream[..self.len].iter_mut().zip(src) {
            *place = item;
            self.prepared += 1;
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return the slice to write to. Unwritten slots may retain previous values.
    #[must_use]
    pub fn slice(&mut self) -> &mut [T] {
        self.prepared = self.len;
        &mut self.stream.as_mut().expect("writer storage")[..self.len]
    }
}

impl<T: Copy> BufferWriter<T> {
    /// Copy samples into the existing write window.
    pub fn fill_from_slice(&mut self, src: &[T]) {
        assert!(
            src.len() <= self.len,
            "trying to write {} samples into a {} sample buffer",
            src.len(),
            self.len
        );
        self.stream.as_mut().expect("writer storage")[..src.len()].copy_from_slice(src);
        self.prepared = src.len();
    }

    /// Commit writes and tags, whose positions are relative to this window.
    pub fn produce(mut self, n: usize, tags: &[Tag]) {
        assert!(
            n <= self.len,
            "trying to produce {n} samples from a {} sample buffer",
            self.len
        );
        assert!(
            n <= self.prepared,
            "trying to produce {n} samples, but only {} samples were written",
            self.prepared
        );
        let mut state = self.parent.state.lock().unwrap();
        state.produce(&self.stream.as_ref().expect("writer storage")[..n], tags);
        state.write_cache = self.stream.take();
    }
}

impl<T> Drop for BufferWriter<T> {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take()
            && let Ok(mut state) = self.parent.state.lock()
        {
            state.write_cache = Some(stream);
        }
    }
}

pub mod export {
    pub(crate) use super::Instant;
    pub(crate) use super::get_cpu_time;
    pub use super::initialize_rustradio;
    pub(crate) use super::sleep;
    pub type Buffer<T> = super::Buffer<T>;
    pub type BufferReader<T> = super::BufferReader<T>;
    pub type BufferWriter<T> = super::BufferWriter<T>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn buffer() -> Arc<Buffer<u32>> {
        Arc::new(Buffer::new(64).unwrap())
    }

    #[test]
    fn writer_storage_is_initialized_once_and_reused_for_smaller_windows() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        #[derive(Clone, Copy)]
        struct Sample(u32);
        impl Default for Sample {
            fn default() -> Self {
                CALLS.fetch_add(1, Ordering::Relaxed);
                Self(0)
            }
        }
        let buffer = Arc::new(Buffer::<Sample>::new(64).unwrap());
        assert_eq!(CALLS.load(Ordering::Relaxed), 32);
        let mut writer = buffer.clone().write_buf().unwrap();
        let allocation = writer.slice().as_ptr();
        writer.slice()[..8].fill(Sample(42));
        writer.produce(8, &[]);
        let mut writer = buffer.clone().write_buf().unwrap();
        assert_eq!(writer.len(), 8);
        assert_eq!(writer.slice().len(), 8);
        assert_eq!(writer.slice().as_ptr(), allocation);
        drop(writer);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert!(reader.iter().all(|sample| sample.0 == 42));
        reader.consume(8);
        let mut writer = buffer.clone().write_buf().unwrap();
        assert_eq!(writer.slice().len(), 16);
        assert_eq!(writer.slice().as_ptr(), allocation);
        assert_eq!(CALLS.load(Ordering::Relaxed), 32);
    }

    #[test]
    fn second_writer_is_rejected_and_drop_returns_storage_without_producing() {
        let buffer = buffer();
        let mut writer = buffer.clone().write_buf().unwrap();
        let allocation = writer.slice().as_ptr();
        writer.slice()[0] = 99;
        assert!(buffer.clone().write_buf().is_err());
        drop(writer);
        assert!(buffer.is_empty());
        let mut writer = buffer.clone().write_buf().unwrap();
        assert_eq!(writer.slice().as_ptr(), allocation);
        writer.produce(0, &[]);
        assert!(buffer.clone().write_buf().is_ok());
    }

    #[test]
    fn read_snapshots_and_writer_copies_preserve_values_across_wraparound() {
        let buffer = buffer();
        buffer.produce(&[0; 14], &[]);
        buffer.consume(14);
        let mut writer = buffer.clone().write_buf().unwrap();
        writer.fill_from_slice(&[11, 12, 13, 14, 15]);
        writer.produce(5, &[]);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice(), [11, 12, 13, 14, 15]);
        let allocation = reader.slice().as_ptr();
        drop(reader);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice().as_ptr(), allocation);
        let mut writer = buffer.clone().write_buf().unwrap();
        writer.fill_from_iter([16, 17]);
        writer.produce(2, &[]);
        // An outstanding reader owns a snapshot, independent of subsequent writes.
        assert_eq!(reader.slice(), [11, 12, 13, 14, 15]);
        reader.consume(3);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice(), [14, 15, 16, 17]);
        reader.consume(4);
        assert!(buffer.is_empty());
    }

    #[test]
    fn fill_helpers_replace_the_prepared_prefix_and_retain_the_allocation() {
        let buffer = buffer();
        let mut writer = buffer.clone().write_buf().unwrap();
        let allocation = writer.slice().as_ptr();
        writer.fill_from_slice(&[1, 2, 3, 4]);
        writer.fill_from_iter([5, 6]);
        assert_eq!(writer.prepared, 2);
        assert_eq!(writer.stream.as_ref().unwrap().as_ptr(), allocation);
        writer.produce(2, &[]);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice(), [5, 6]);
        reader.consume(2);
        let mut writer = buffer.clone().write_buf().unwrap();
        writer.fill_from_iter(0..100);
        assert_eq!(writer.prepared, 16);
        writer.fill_from_slice(&[7]);
        assert_eq!(writer.prepared, 1);
        writer.produce(1, &[]);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice(), [7]);
        reader.consume(1);
    }

    #[test]
    fn reused_storage_does_not_allow_producing_an_unprepared_prefix() {
        let buffer = buffer();
        let mut writer = buffer.clone().write_buf().unwrap();
        writer.slice().fill(42);
        drop(writer);
        let writer = buffer.clone().write_buf().unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| writer.produce(1, &[]))).is_err());
        let mut writer = buffer.clone().write_buf().unwrap();
        writer.fill_from_iter([1, 2]);
        assert!(catch_unwind(AssertUnwindSafe(|| writer.produce(3, &[]))).is_err());
        assert!(buffer.is_empty());
        assert!(buffer.clone().write_buf().is_ok());
    }

    #[test]
    fn reader_drop_reuses_storage_without_consuming() {
        let buffer = buffer();
        buffer.produce(&[1, 2, 3, 4], &[]);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        let allocation = reader.slice().as_ptr();
        drop(reader);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice(), [1, 2, 3, 4]);
        assert_eq!(reader.slice().as_ptr(), allocation);
        reader.consume(0);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert_eq!(reader.slice().as_ptr(), allocation);
        reader.consume(4);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        assert!(reader.is_empty());
        reader.consume(0);
    }

    #[test]
    fn recycling_skips_poisoned_state_during_unwinding() {
        let buffer = buffer();
        buffer.produce(&[1], &[]);
        let (reader, _) = buffer.clone().read_buf().unwrap();
        let mut writer = buffer.clone().write_buf().unwrap();
        writer.fill_from_slice(&[2]);
        let tags = [Tag::new(1, "invalid", crate::stream::TagValue::Bool(true))];
        assert!(catch_unwind(AssertUnwindSafe(|| writer.produce(1, &tags))).is_err());
        assert!(buffer.state.is_poisoned());
        drop(reader);
    }
}
