//! Test implementation of circular buffers.
//! Full of unsafe. Full of ugly code.
//!
// TODO:
// * Make Circ typed?

use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Condvar, Mutex};

use libc::{MAP_FAILED, MAP_FIXED, MAP_SHARED, PROT_READ, PROT_WRITE};
use libc::{c_uchar, c_void, size_t};
use log::error;

use crate::stream::{Tag, TagPos};
use crate::{Error, Result};

const SYNC_SLEEP_TIME: std::time::Duration = std::time::Duration::from_millis(100);
#[cfg(feature = "async")]
const ASYNC_SLEEP_TIME: tokio::time::Duration = tokio::time::Duration::from_millis(100);

#[derive(Debug)]
struct Map {
    base: *mut c_uchar,
    len: usize,
}

impl Map {
    fn new(f: &std::fs::File, len: usize) -> Result<Self> {
        Self::with_addr(f, len, std::ptr::null_mut())
    }
    // TODO: change ptr to be Option<*mut c_void>.
    // Null pointer enum optimization.
    fn with_addr(f: &std::fs::File, len: usize, ptr: *mut c_void) -> Result<Self> {
        let fd = f.as_raw_fd();
        let flags = MAP_SHARED | if ptr.is_null() { 0 } else { MAP_FIXED };

        // SAFETY:
        // * If non-fixed: worst case we'll leak memory if this function
        //   doesn't handle errors properly.
        // * If fixed: caller *must not* call this on just any place in memory.
        //
        // TODO:
        // * Verify pointer alignment with page boundary.
        // * Replace this function with a Map `.split()`, so that the fixed
        //   pointer assumption is restricted to Map.
        let buf = unsafe { libc::mmap(ptr, len as size_t, PROT_READ | PROT_WRITE, flags, fd, 0) };
        if std::ptr::eq(buf, MAP_FAILED) {
            let e = std::io::Error::last_os_error();
            return Err(Error::msg(format!(
                "mmap(){}: {e}",
                if ptr.is_null() {
                    ""
                } else {
                    " at fixed address"
                }
            )));
        }
        assert!(!buf.is_null());
        if !ptr.is_null() && !std::ptr::eq(ptr, buf) {
            // SAFETY: we literally just allocated using this pointer and
            // length, so this has to be fine.
            let rc = unsafe { libc::munmap(buf, len as size_t) };
            if rc != 0 {
                let e = std::io::Error::last_os_error();
                panic!("Failed to unmap buffer just mapped in the failure path: {e}");
            }
            return Err(Error::msg("mmap() allocated in the wrong place"));
        }
        Ok(Self {
            base: buf as *mut c_uchar,
            len,
        })
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: This is what we mmapped.
        let rc = unsafe { libc::munmap(self.base as *mut c_void, self.len) };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            panic!("munmap() failed on circular buffer: {e}");
        }
    }
}

/// Circular buffer dealing in bytes.
#[derive(Debug)]
pub struct Circ {
    len: usize,
    map: Map,
    _map2: Map, // Held on to for the Drop.
}

impl Circ {
    /// Create a new circular buffer.
    fn new(size: usize) -> Result<Self> {
        let size_x2 = size * 2;
        // Annotating the temp dir directory may help in case of not enough
        // space, permissions, etc.
        let errfix = |e| Error::file_io(e, std::env::temp_dir());
        let f = tempfile::tempfile().map_err(errfix)?;
        f.set_len(size_x2 as u64)?;

        // Map first half.
        let mut map = Map::new(&f, size_x2)?;

        // Remap second half to be same as the first.
        // Be very careful with the order, here.
        let second = (map.base as libc::uintptr_t + size as libc::uintptr_t) as *mut c_void;
        let map2 = Map::with_addr(&f, size, second)?;

        // First map is now just `size`.
        map.len = size;

        // Shrink file.
        f.set_len(size as u64).map_err(errfix)?;

        Ok(Self {
            len: size_x2,
            map,
            _map2: map2,
        })
    }

    /// Return length of buffer, *before* the double mapping, in bytes.
    #[must_use]
    pub fn total_size(&self) -> usize {
        // self.len is number of bytes in the entire buffer. The
        // mapping is 2x the writable size when the buffer is empty,
        // which is what's relevant to callers.
        self.len / 2
    }

    // I'm pretty sure this is a safe error to suppress. Clippy is not
    // wrong, it's scary. But this whole thing is scary unsafe.
    //
    // Possibly the compiler sees something UB, and breaks things with
    // a strange optimization, but let's hope not. :-)
    //
    // The reason this function asserts instead of returns error is that it's
    // only called from this module, and "cannot" be called with invalid
    // arguments.
    #[allow(clippy::mut_from_ref)]
    #[must_use]
    fn full_buffer<T>(&self, start: usize, end: usize) -> &mut [T] {
        let ez = std::mem::size_of::<T>();
        debug_assert!(self.len.is_multiple_of(ez));
        debug_assert!(
            end - start <= self.len / ez / 2,
            "requested {start} to {end} ({} entries) of {} but len is {}",
            end - start,
            ez,
            self.len
        );
        // SAFETY: This is a mut cast for memory that C++ would call non-pointer
        // POD. Data races are possible from this in general, but will be
        // prevented by the stream API.
        let buf = unsafe { std::slice::from_raw_parts_mut(self.map.base as *mut T, self.len / ez) };
        &mut buf[start..end]
    }
}

// SAFETY: Circ are just metadata around normal process-local memory.
unsafe impl Send for Circ {}
// SAFETY: Circ are just metadata around normal process-local memory.
unsafe impl Sync for Circ {}

#[derive(Debug)]
struct BufferState {
    rpos: usize,        // In samples.
    wpos: usize,        // In samples.
    used: usize,        // In samples.
    circ_len: usize,    // In bytes.
    member_size: usize, // In bytes.
    tags: BTreeMap<TagPos, Vec<Tag>>,
}

impl BufferState {
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

    /// Total capacity of this buffer, ignoring fullness. In samples.
    #[must_use]
    fn capacity(&self) -> usize {
        self.circ_len / self.member_size
    }

    /// How many samples fit to be written.
    #[must_use]
    fn write_capacity(&self) -> usize {
        let (a, b) = self.write_range();
        b - a
    }

    /// Available space to be written, in samples.
    #[must_use]
    fn free(&self) -> usize {
        self.capacity() - self.used
    }
}

/// BufferReader is an RAII'd fixed window read slice with some helper functions.
pub struct BufferReader<T> {
    parent: Arc<Buffer<T>>,
    start: usize,
    end: usize,
}

impl<T: Copy> BufferReader<T> {
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
        self.end - self.start
    }

    /// is_empty convenience function.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

impl<T: Copy> std::ops::Index<usize> for BufferReader<T> {
    type Output = T;

    fn index(&self, n: usize) -> &Self::Output {
        &self.slice()[n]
    }
}

/// BufferWriter is an RAII fixed window slice with some helper functions.
pub struct BufferWriter<T> {
    parent: Arc<Buffer<T>>,
    start: usize,
    end: usize,
}

impl<T: Copy> BufferWriter<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> BufferWriter<T> {
        assert!(end >= start);
        Self { parent, start, end }
    }

    /// Return the slice to write to.
    #[must_use]
    pub fn slice(&mut self) -> &mut [T] {
        self.parent.slice_mut(self.start, self.end)
    }

    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_iter(&mut self, src: impl IntoIterator<Item = T>) {
        for (place, item) in self.slice().iter_mut().zip(src) {
            *place = item;
        }
    }

    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_slice(&mut self, src: &[T]) {
        self.slice()[..src.len()].copy_from_slice(src);
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
        self.end - self.start
    }

    /// is_empty convenience function.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

#[derive(Debug)]
struct BufferInner {
    lock: Mutex<BufferState>,
    cv: Condvar,

    // Waiting for read.
    #[cfg(feature = "async")]
    acvr: tokio::sync::Notify,

    // Waiting for write.
    #[cfg(feature = "async")]
    acvw: tokio::sync::Notify,
}

/// Type aware buffer.
#[derive(Debug)]
pub struct Buffer<T> {
    id: usize,
    state: Arc<BufferInner>,
    circ: Circ,
    member_size: usize,
    dummy: std::marker::PhantomData<T>,
}

impl<T> Buffer<T> {
    /// Create a new Buffer.
    pub fn new(size: usize) -> Result<Self> {
        Ok(Self {
            id: crate::NEXT_STREAM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            state: Arc::new(BufferInner {
                lock: Mutex::new(BufferState {
                    rpos: 0,
                    wpos: 0,
                    used: 0,
                    circ_len: size,
                    member_size: std::mem::size_of::<T>(),
                    tags: BTreeMap::new(),
                }),
                cv: Condvar::new(),
                #[cfg(feature = "async")]
                acvr: tokio::sync::Notify::new(),
                #[cfg(feature = "async")]
                acvw: tokio::sync::Notify::new(),
            }),
            member_size: std::mem::size_of::<T>(),
            circ: Circ::new(size)?,
            dummy: std::marker::PhantomData,
        })
    }

    #[must_use]
    pub(crate) fn id(&self) -> usize {
        self.id
    }

    /// Return length of buffer, ignoring how much is in use, and the
    /// double buffer.
    #[must_use]
    pub fn total_size(&self) -> usize {
        self.circ.total_size() / self.member_size
    }

    /// Available space to write, in bytes.
    #[must_use]
    pub fn free(&self) -> usize {
        self.state.lock.lock().unwrap().free()
    }
    pub fn wait_for_write(&self, need: usize) -> usize {
        self.state
            .cv
            .wait_timeout_while(self.state.lock.lock().unwrap(), SYNC_SLEEP_TIME, |s| {
                s.free() < need
            })
            .unwrap()
            .0
            .free()
    }
    #[cfg(feature = "async")]
    pub async fn wait_for_write_async(&self, _need: usize) -> usize {
        // TODO: loop or something.
        let sleep = tokio::time::sleep(ASYNC_SLEEP_TIME);
        tokio::select! {
            _ = sleep => 0,
            _ = self.state.acvw.notified() => 1,
        }
    }
    pub fn wait_for_read(&self, need: usize) -> usize {
        self.state
            .cv
            .wait_timeout_while(self.state.lock.lock().unwrap(), SYNC_SLEEP_TIME, |s| {
                s.used < need
            })
            .unwrap()
            .0
            .used
    }
    #[cfg(feature = "async")]
    pub async fn wait_for_read_async(&self, _need: usize) -> usize {
        // TODO: loop or something.
        let sleep = tokio::time::sleep(ASYNC_SLEEP_TIME);
        tokio::select! {
            _ = sleep => 0,
            _ = self.state.acvr.notified() => 1,
        }
    }
}

impl<T> Buffer<T> {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        let state = self.state.lock.lock().unwrap();
        state.used == 0
    }
}

impl<T: Copy> Buffer<T> {
    /// Consume samples from input buffer.
    ///
    /// Will only be called from the read buffer.
    pub(in crate::circular_buffer) fn consume(&self, n: usize) {
        let mut s = self.state.lock.lock().unwrap();
        assert!(
            n <= s.used,
            "trying to consume {}, but only have {}",
            n,
            s.used
        );
        let newpos = (s.rpos + n) % s.capacity();
        use std::ops::Bound::{Excluded, Included};

        let keys: Vec<TagPos> = if newpos > s.rpos {
            s.tags
                .range((Included(s.rpos), Excluded(newpos)))
                .map(|(k, _)| *k)
                .collect()
        } else {
            let mut t: Vec<TagPos> = s
                .tags
                .range((Included(s.rpos), Excluded(s.capacity())))
                .map(|(k, _)| *k)
                .collect();
            t.extend(
                s.tags
                    .range((Included(0), Excluded(newpos)))
                    .map(|(k, _)| *k),
            );
            t
        };
        for k in keys {
            s.tags.remove(&k);
        }
        s.rpos = newpos;
        s.used -= n;
        self.state.cv.notify_all();
        #[cfg(feature = "async")]
        self.state.acvw.notify_one();
    }

    /// Produce samples (commit writes).
    ///
    /// Will only be called from the write buffer.
    pub(in crate::circular_buffer) fn produce(&self, n: usize, tags: &[Tag]) {
        if n == 0 {
            debug_assert!(tags.is_empty());
            if !tags.is_empty() {
                error!("produce() called on a stream with 0 entries, but non-empty tags: {tags:?}");
            }
            return;
        }
        let mut s = self.state.lock.lock().unwrap();
        assert!(
            s.free() >= n,
            "tried to produce {n}, but only {} is free out of {}",
            s.free(),
            self.total_size()
        );
        assert!(
            s.write_capacity() >= n,
            "can't produce that much. {} < {}",
            s.write_capacity(),
            n
        );
        for tag in tags {
            let pos = (tag.pos() + s.wpos) % s.capacity();
            let tag = Tag::new(pos, tag.key(), tag.val().clone());
            s.tags.entry(pos).or_default().push(tag);
        }
        s.wpos = (s.wpos + n) % s.capacity();
        s.used += n;
        self.state.cv.notify_all();
        #[cfg(feature = "async")]
        self.state.acvr.notify_waiters();
    }

    #[must_use]
    pub(crate) fn slice(&self, start: usize, end: usize) -> &[T] {
        self.circ.full_buffer::<T>(start, end)
    }

    #[must_use]
    pub(crate) fn slice_mut(&self, start: usize, end: usize) -> &mut [T] {
        self.circ.full_buffer::<T>(start, end)
    }

    /// Get the read slice.
    ///
    /// TODO: no need for Result in API.
    pub fn read_buf(self: Arc<Self>) -> Result<(BufferReader<T>, Vec<Tag>)> {
        let s = self.state.lock.lock().unwrap();
        let (start, end) = s.read_range();
        let mut tags = Vec::with_capacity(s.tags.len());

        // TODO: range scan the tags.
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
        tags.sort_by_key(|a| a.pos());
        Ok((BufferReader::new(self, start, end), tags))
    }

    /// Get the write slice.
    pub fn write_buf(self: Arc<Self>) -> Result<BufferWriter<T>> {
        let s = self.state.lock.lock().unwrap();
        let (start, end) = s.write_range();
        drop(s);
        Ok(BufferWriter::new(
            //unsafe { std::mem::transmute::<&mut [T], &mut [T]>(buf) },
            self, start, end,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Float;
    use crate::stream::TagValue;

    #[test]
    fn circ_reqlen() -> Result<()> {
        let circ = Circ::new(4096)?;
        assert_eq!(circ.total_size(), 4096);
        assert!(circ.full_buffer::<u32>(0, 0).is_empty());
        assert_eq!(circ.full_buffer::<u32>(0, 1).len(), 1);
        assert_eq!(circ.full_buffer::<u32>(0, 1024).len(), 1024);
        assert_eq!(circ.full_buffer::<u32>(1000, 1200).len(), 200);
        assert_eq!(circ.full_buffer::<u32>(2040, 2048).len(), 8);
        Ok(())
    }

    #[test]
    #[should_panic]
    fn circ_reqlen_too_big_beginning() {
        if let Ok(circ) = Circ::new(4096) {
            let _ = circ.full_buffer::<u32>(0, 1025);
        }
    }

    #[test]
    #[should_panic]
    fn circ_reqlen_too_big_middle() {
        if let Ok(circ) = Circ::new(4096) {
            let _ = circ.full_buffer::<u32>(10, 1024 + 11);
        }
    }

    #[test]
    #[should_panic]
    fn circ_past_end() {
        if let Ok(circ) = Circ::new(4096) {
            let _ = circ.full_buffer::<u32>(2040, 2049);
        }
    }

    #[test]
    fn circ_circular() -> Result<()> {
        let circ = Circ::new(4096)?;
        assert_eq!(circ.total_size(), 4096);
        let buf = circ.full_buffer::<u32>(0, 4);
        let buf2 = circ.full_buffer::<u32>(1024, 1028);
        assert_eq!(buf, [0, 0, 0, 0]);
        assert_eq!(buf2, [0, 0, 0, 0]);
        buf[0] = 3;
        buf[1] = 2;
        buf[2] = 1;
        buf[3] = 42;
        // Not sure if compiler fence is enough here, or we need a full `fence`.
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(buf, [3, 2, 1, 42]);
        assert_eq!(buf2, [3, 2, 1, 42]);
        assert_ne!(buf.as_ptr(), buf2.as_ptr());
        Ok(())
    }

    #[test]
    fn typical() -> Result<()> {
        let b = Arc::new(Buffer::new(4096)?);

        // Initial.
        assert!(b.clone().read_buf()?.0.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 4096);

        // Write a byte.
        {
            let mut buf = b.clone().write_buf()?;
            buf.slice()[0] = 123;
            buf.produce(1, &[Tag::new(0, "start", TagValue::Bool(true))]);
            assert_eq!(b.clone().read_buf()?.0.slice(), vec![123]);
            assert_eq!(
                b.clone().read_buf()?.1,
                vec![Tag::new(0, "start", TagValue::Bool(true))]
            );
            assert_eq!(b.clone().write_buf()?.len(), 4095);
        }

        // Consume the byte.
        b.consume(1);
        assert!(b.clone().read_buf()?.0.is_empty());
        assert!(b.clone().read_buf()?.1.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 4096);

        // Write towards the end bytes.
        {
            let n = 4000;
            let mut wb = b.clone().write_buf()?;
            for i in 0..n {
                wb.slice()[i] = (i & 0xff) as u8;
            }
            wb.produce(n, &[Tag::new(1, "foo", TagValue::String("bar".into()))]);
            let (rb, rt) = b.clone().read_buf()?;
            assert_eq!(rb.len(), n);
            for i in 0..n {
                assert_eq!(rb.slice()[i], (i & 0xff) as u8);
            }
            assert_eq!(rt, vec![Tag::new(1, "foo", TagValue::String("bar".into()))]);
            assert_eq!(b.clone().write_buf()?.len(), 4096 - n);
        }
        b.consume(4000);

        // Write 100 bytes.
        {
            let n = 100;
            let mut wb = b.clone().write_buf()?;
            for i in 0..n {
                wb.slice()[i] = ((n - i) & 0xff) as u8;
            }
            wb.produce(
                n,
                &[
                    Tag::new(0, "first", TagValue::Bool(true)),
                    Tag::new(99, "last", TagValue::Bool(false)),
                ],
            );
            let (rb, rt) = b.clone().read_buf()?;
            assert_eq!(rb.len(), n);
            for i in 0..n {
                assert_eq!(rb.slice()[i], ((n - i) & 0xff) as u8);
            }
            assert_eq!(
                rt,
                vec![
                    Tag::new(0, "first", TagValue::Bool(true)),
                    Tag::new(99, "last", TagValue::Bool(false))
                ]
            );
            drop(rb);
            assert_eq!(b.clone().read_buf()?.0.len(), 100);
            assert_eq!(b.clone().write_buf()?.len(), 3996);
        }

        // Clear it.
        {
            let (rb, _) = b.clone().read_buf()?;
            let n = rb.len();
            rb.consume(n);
            assert_eq!(b.clone().read_buf()?.0.len(), 0);
            assert!(b.clone().read_buf()?.1.is_empty());
            assert_eq!(b.clone().write_buf()?.len(), 4096);
        }
        Ok(())
    }

    #[test]
    fn two_writes() -> Result<()> {
        let b: Arc<Buffer<u8>> = Arc::new(Buffer::new(4096)?);

        // Write 10 bytes.
        {
            let mut buf = b.clone().write_buf()?;
            buf.slice()[1] = 123;
            buf.produce(10, &[Tag::new(1, "first", TagValue::Bool(true))]);
            assert_eq!(
                b.clone().read_buf()?.0.slice(),
                vec![0, 123, 0, 0, 0, 0, 0, 0, 0, 0]
            );
            assert_eq!(
                b.clone().read_buf()?.1,
                vec![Tag::new(1, "first", TagValue::Bool(true))]
            );
            assert_eq!(b.clone().write_buf()?.len(), 4086);
        }

        // Write 5 more bytes.
        {
            let mut buf = b.clone().write_buf()?;
            buf.slice()[2] = 42;
            buf.produce(5, &[Tag::new(2, "second", TagValue::Bool(false))]);
            assert_eq!(
                b.clone().read_buf()?.0.slice(),
                vec![0, 123, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 42, 0, 0]
            );
            assert_eq!(
                b.clone().read_buf()?.1,
                vec![
                    Tag::new(1, "first", TagValue::Bool(true)),
                    Tag::new(12, "second", TagValue::Bool(false))
                ]
            );
            assert_eq!(b.clone().write_buf()?.len(), 4081);
        }

        // Consume the byte.
        b.consume(15);
        assert!(b.clone().read_buf()?.0.is_empty());
        assert!(b.clone().read_buf()?.1.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 4096);
        Ok(())
    }

    #[test]
    fn exact_overflow() -> Result<()> {
        let b: Arc<Buffer<u8>> = Arc::new(Buffer::new(4096)?);

        // Initial.
        assert!(b.clone().read_buf()?.0.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 4096);

        // Full.
        b.clone().write_buf()?.produce(4096, &[]);
        assert_eq!(b.clone().read_buf()?.0.len(), 4096);
        assert_eq!(b.clone().write_buf()?.len(), 0);

        // Empty again.
        b.clone().read_buf()?.0.consume(4096);
        assert!(b.clone().read_buf()?.0.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 4096);
        Ok(())
    }

    #[test]
    fn with_float() -> Result<()> {
        let b: Arc<Buffer<Float>> = Arc::new(Buffer::new(4096)?);

        // Initial.
        assert!(b.clone().read_buf()?.0.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 1024);

        // Write a sample.
        {
            let mut wb = b.clone().write_buf()?;
            wb.slice()[0] = 123.321;
            wb.produce(1, &[]);
        }
        assert_eq!(b.clone().read_buf()?.0.slice(), vec![123.321]);
        assert_eq!(b.clone().write_buf()?.len(), 1023);

        // Consume the sample.
        b.clone().read_buf()?.0.consume(1);
        assert!(b.clone().read_buf()?.0.is_empty());
        assert_eq!(b.clone().write_buf()?.len(), 1024);

        // Write towards the end bytes.
        {
            let n = 1000;
            let mut wb = b.clone().write_buf()?;
            for i in 0..n {
                wb.slice()[i] = i as Float;
            }
            wb.produce(n, &[]);
            assert_eq!(b.clone().read_buf()?.0.len(), n);
            for i in 0..n {
                assert_eq!(b.clone().read_buf()?.0.slice()[i], i as Float);
            }
            assert_eq!(b.clone().write_buf()?.len(), 24);
        }
        b.clone().read_buf()?.0.consume(1000);

        // Write 100 bytes.
        {
            let n = 100;
            let mut wb = b.clone().write_buf()?;
            for i in 0..n {
                wb.slice()[i] = (n - i) as Float;
            }
            wb.produce(n, &[]);
            assert_eq!(b.clone().read_buf()?.0.len(), n);
            for i in 0..n {
                assert_eq!(b.clone().read_buf()?.0.slice()[i], (n - i) as Float);
            }
        }
        assert_eq!(b.clone().read_buf()?.0.len(), 100);
        assert_eq!(b.clone().write_buf()?.len(), 1024 - 100);
        Ok(())
    }
}
/* vim: textwidth=80
 */
