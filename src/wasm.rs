//! This module contains wasm versions of various code.
//!
//! It must fail gracefully when used in a web worker.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate::Result;
use crate::stream::{Tag, TagPos};

#[must_use]
pub(crate) fn get_cpu_time() -> std::time::Duration {
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

#[derive(Debug)]
struct BufferState<T> {
    rpos: usize,
    wpos: usize,
    used: usize,
    stream: Vec<T>,
    tags: BTreeMap<TagPos, Vec<Tag>>,
}
impl<T> BufferState<T> {
    fn new(size: usize) -> Self {
        let mut stream = Vec::with_capacity(size);
        (0..size).for_each(|_| {
            // TODO: There should be a better way. But we can't demand `Default`
            // trait, since that would infect so much other code.
            // SAFETY: This should be fine since T's are all ints and floats.
            let val = unsafe { std::mem::zeroed() };
            stream.push(val);
        });
        Self {
            rpos: 0,
            wpos: 0,
            used: 0,
            stream,
            tags: BTreeMap::default(),
        }
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
}
impl<T> BufferState<T> {
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
            state: Mutex::new(BufferState::new(size)),
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
    pub(crate) fn len(&self) -> usize {
        self.state.lock().unwrap().used
    }
    /// Available space to write, in bytes(?).
    pub(crate) fn free(&self) -> usize {
        self.state.lock().unwrap().free()
    }
    pub(crate) fn slice(&self, start: usize, end: usize) -> &[T] {
        self.slice_mut(start, end)
    }
    pub(crate) fn slice_mut(&self, start: usize, end: usize) -> &mut [T] {
        unsafe {
            let l = self.state.lock().unwrap();
            let ptr = l.stream.as_ptr() as *mut T;
            std::slice::from_raw_parts_mut(ptr.add(start), end - start)
        }
    }
    pub fn consume(&self, n: usize) {
        let mut l = self.state.lock().unwrap();
        l.rpos = (l.rpos + n) % l.size();
        l.used -= n;
    }
    pub fn produce(&self, n: usize, tags: &[Tag]) {
        let mut l = self.state.lock().unwrap();
        for tag in tags {
            let pos = (tag.pos() + l.wpos) % l.capacity();
            let tag = Tag::new(pos, tag.key(), tag.val().clone());
            l.tags.entry(pos).or_default().push(tag);
        }
        l.wpos = (l.wpos + n) % l.size();
        l.used += n;
    }
    pub fn total_size(&self) -> usize {
        self.len()
    }
    pub fn read_buf(self: Arc<Self>) -> Result<(BufferReader<T>, Vec<Tag>)> {
        let s = self.state.lock().unwrap();
        let (start, end) = s.read_range();
        let mut tags = Vec::with_capacity(s.tags.len());
        for (n, ts) in &s.tags {
            let modded_n: usize = *n % s.capacity();
            if end < s.capacity() && start < s.capacity() {
                // Start and end are both in first half.
                if modded_n < start || modded_n > end {
                    continue;
                }
            } else {
                // Start and end can't both be in the second half, and
                // end has to be higher than start.
                assert!(start < s.capacity());
                if modded_n > (end % s.capacity()) && modded_n < start {
                    continue;
                }
            }
            for tag in ts {
                tags.push(Tag::new(
                    (tag.pos() + s.capacity() - start) % s.capacity(),
                    tag.key(),
                    tag.val().clone(),
                ));
            }
        }
        drop(s);
        tags.sort_by_key(Tag::pos);
        Ok((BufferReader::new(self, start, end), tags))
    }
    pub fn write_buf(self: Arc<Self>) -> Result<BufferWriter<T>> {
        let l = self.state.lock().unwrap();
        let (start, end) = l.write_range();
        drop(l);
        Ok(BufferWriter::new(self, start, end))
    }
    pub fn wait_for_write(&self, _need: usize) -> usize {
        // TODO
        1
    }
    pub fn wait_for_read(&self, _need: usize) -> usize {
        // TODO
        1
    }
}

pub struct BufferReader<T> {
    parent: Arc<Buffer<T>>,
    start: usize,
    end: usize,
}
impl<T> BufferReader<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> Self {
        Self { parent, start, end }
    }

    /// Return slice to read from.
    #[must_use]
    pub fn slice(&self) -> &[T] {
        self.parent.slice(self.start, self.end)
    }

    /// Helper function to iterate over input instead.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.slice().iter()
    }

    /// We're done with the buffer. Consume `n` samples.
    pub fn consume(self, n: usize) {
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
    start: usize,
    end: usize,
}
impl<T> BufferWriter<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> BufferWriter<T> {
        Self { parent, start, end }
    }

    /// Return the slice to write to.
    #[must_use]
    pub fn slice(&mut self) -> &mut [T] {
        self.parent.slice_mut(self.start, self.end)
    }
}
impl<T: Copy> BufferWriter<T> {
    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_slice(&mut self, src: &[T]) {
        self.slice()[..src.len()].copy_from_slice(src);
    }

    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_iter(&mut self, src: impl IntoIterator<Item = T>) {
        for (place, item) in self.slice().iter_mut().zip(src) {
            *place = item;
        }
    }

    /// Having written into the write buffer, now tell the buffer
    /// we're done. Also here are the tags, with positions relative to
    /// start of buffer.
    ///
    // Tags inherently need to be copied in, because they need to be added to
    // the underlying stream.
    pub fn produce(self, n: usize, tags: &[Tag]) {
        self.parent.produce(n, tags);
    }

    /// len convenience function.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parent.slice(self.start, self.end).len()
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
    pub(crate) use super::sleep;
    pub type Buffer<T> = super::Buffer<T>;
    pub type BufferReader<T> = super::BufferReader<T>;
    pub type BufferWriter<T> = super::BufferWriter<T>;
}
