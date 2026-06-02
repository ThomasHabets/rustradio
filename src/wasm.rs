//! This module contains wasm versions of various code.
//!
//! It must fail gracefully when used in a web worker.
use std::collections::BTreeMap;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::Mutex;

use wasm_bindgen::prelude::*;

use crate::stream::{Tag, TagPos};
use crate::{Error, Result};

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

// The stream in BufferState is not actually shared. Producing initializes
// values in it, and consuming marks the slots free by advancing rpos/used.
//
// This is not as performant as the circular buffer for non-WASM, but it does
// work.
//
// Originally this used `Vec<Option<T>>`, but that uses twice the buffer space
// and was marginally slower. (an AX.25 decode test went from ~60% CPU to ~55%).
//
// It should be possible to not copy to and from the readers and writers, but it
// requires more careful lifetime and pointer handling.
//
// The main requirement making this complex is that the users of these buffers
// need linear `&[T]` to work with, and a block needing to write two elements can
// get stuck if we keep giving it just one elements of space.
//
// We can't do `VecDeque` because it doesn't give us a linear buffer.
//
// We can't "just" rotate the buffer when needed. Well, we can, but:
// 1. We need to make sure there are no readers or writers outstanding, and
// 2. every rotation means copying all the elements, which is what we wanted to
//    avoid in the first place. Though to be fair, one less copy.
#[derive(Debug)]
struct BufferState<T> {
    rpos: usize,
    wpos: usize,
    used: usize,
    // Only the range described by rpos/used is initialized.
    stream: Vec<MaybeUninit<T>>,
    tags: BTreeMap<TagPos, Vec<Tag>>,
}
impl<T> BufferState<T> {
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
        let mut stream = Vec::with_capacity(size);
        stream.resize_with(size, MaybeUninit::uninit);
        Ok(Self {
            rpos: 0,
            wpos: 0,
            used: 0,
            stream,
            tags: BTreeMap::default(),
        })
    }
    // Return write range, in samples.
    #[must_use]
    fn write_range(&self) -> (usize, usize) {
        //eprintln!("Write range: {} {}", self.rpos, self.wpos);
        (self.wpos, self.wpos + self.free())
    }
    // Read range, in samples
    #[must_use]
    fn read_range(&self) -> (usize, usize) {
        (self.rpos, self.rpos + self.used)
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
impl<T> Buffer<T> {
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
        eprintln!("BLEH: {:?}", self.state.lock().unwrap().used);
        self.state.lock().unwrap().used == 0
    }
    /// Available space to write, in bytes(?).
    pub(crate) fn free(&self) -> usize {
        self.state.lock().unwrap().free()
    }
    pub fn consume(&self, n: usize) {
        let mut l = self.state.lock().unwrap();
        assert!(
            n <= l.used,
            "trying to consume {n}, but only have {}",
            l.used
        );
        let capacity = l.capacity();
        for i in 0..n {
            let pos = (l.rpos + i) % capacity;
            l.tags.remove(&pos);
        }
        l.rpos = (l.rpos + n) % capacity;
        l.used -= n;
    }
    pub fn total_size(&self) -> usize {
        self.state.lock().unwrap().capacity()
    }
    pub fn wait_for_write(&self, _need: usize) -> usize {
        // TODO
        1
    }
    pub fn wait_for_read(&self, _need: usize) -> usize {
        // TODO
        1
    }
    #[cfg(feature = "async")]
    pub async fn wait_for_write_async(&self, _need: usize) -> usize {
        // TODO
        1
    }
    #[cfg(feature = "async")]
    pub async fn wait_for_read_async(&self, _need: usize) -> usize {
        // TODO
        1
    }
}

impl<T: Copy> Buffer<T> {
    pub fn produce(&self, samples: &[T], tags: &[Tag]) {
        if samples.is_empty() {
            debug_assert!(tags.is_empty());
            return;
        }
        let mut l = self.state.lock().unwrap();
        assert!(
            samples.len() <= l.free(),
            "tried to produce {}, but only {} is free out of {}",
            samples.len(),
            l.free(),
            l.capacity()
        );
        let capacity = l.capacity();
        let wpos = l.wpos;
        for (i, sample) in samples.iter().copied().enumerate() {
            l.stream[(wpos + i) % capacity].write(sample);
        }
        for tag in tags {
            let pos = (tag.pos() + wpos) % capacity;
            let tag = Tag::new(pos, tag.key(), tag.val().clone());
            l.tags.entry(pos).or_default().push(tag);
        }
        l.wpos = (wpos + samples.len()) % capacity;
        l.used += samples.len();
    }
    pub fn read_buf(self: Arc<Self>) -> Result<(BufferReader<T>, Vec<Tag>)> {
        let s = self.state.lock().unwrap();
        let (start, end) = s.read_range();
        let used = end - start;
        let capacity = s.capacity();
        let mut stream = Vec::with_capacity(used);
        for i in 0..used {
            let pos = (start + i) % capacity;
            // SAFETY: BufferState maintains the invariant that exactly the
            // slots in the read range described by rpos/used are initialized.
            //
            // This used to be an `Option` with `expect`, and it never
            // triggered.
            stream.push(unsafe { s.stream[pos].assume_init() });
        }
        let mut tags = Vec::with_capacity(s.tags.len());
        for (n, ts) in &s.tags {
            let relative_pos = (*n + capacity - start) % capacity;
            if relative_pos >= used {
                continue;
            }
            for tag in ts {
                tags.push(Tag::new(relative_pos, tag.key(), tag.val().clone()));
            }
        }
        drop(s);
        tags.sort_by_key(Tag::pos);
        Ok((BufferReader::new(self, stream), tags))
    }
    pub fn write_buf(self: Arc<Self>) -> Result<BufferWriter<T>> {
        let l = self.state.lock().unwrap();
        let (start, end) = l.write_range();
        drop(l);
        Ok(BufferWriter::new(self, end - start))
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
    pub fn consume(self, n: usize) {
        assert!(
            n <= self.stream.len(),
            "trying to consume {n}, but read buffer only has {}",
            self.stream.len()
        );
        self.parent.consume(n);
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
pub struct BufferWriter<T> {
    parent: Arc<Buffer<T>>,
    len: usize,
    stream: Vec<T>,
}
impl<T> BufferWriter<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, len: usize) -> BufferWriter<T> {
        Self {
            parent,
            len,
            stream: Vec::new(),
        }
    }
}

impl<T: Default> BufferWriter<T> {
    /// Return the slice to write to.
    #[must_use]
    pub fn slice(&mut self) -> &mut [T] {
        if self.stream.len() < self.len {
            self.stream.resize_with(self.len, T::default);
        }
        self.stream.as_mut_slice()
    }
}
impl<T: Copy> BufferWriter<T> {
    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_slice(&mut self, src: &[T]) {
        assert!(
            src.len() <= self.len,
            "trying to write {} samples into a {} sample buffer",
            src.len(),
            self.len
        );
        self.stream = src.to_vec();
    }

    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_iter(&mut self, src: impl IntoIterator<Item = T>) {
        self.stream = src.into_iter().take(self.len).collect();
    }

    /// Having written into the write buffer, now tell the buffer
    /// we're done. Also here are the tags, with positions relative to
    /// start of buffer.
    ///
    // Tags inherently need to be copied in, because they need to be added to
    // the underlying stream.
    pub fn produce(self, n: usize, tags: &[Tag]) {
        assert!(
            n <= self.len,
            "trying to produce {n} samples from a {} sample buffer",
            self.len
        );
        if n == 0 {
            debug_assert!(tags.is_empty(), "produced 0 samples with nonzero tags");
            return;
        }
        assert!(
            n <= self.stream.len(),
            "trying to produce {n} samples, but only {} samples were written",
            self.stream.len()
        );
        self.parent.produce(&self.stream[..n], tags);
    }

    /// len convenience function.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// is_empty convenience function.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
