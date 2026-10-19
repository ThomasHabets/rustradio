//! Test implementation of circular buffers.
//! Full of unsafe. Full of ugly code.
//!
// TODO:
// * Make Circ typed?

use std::borrow::Cow;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Condvar, Mutex};

use libc::{MAP_FAILED, MAP_FIXED, MAP_SHARED, PROT_READ, PROT_WRITE};
use libc::{c_uchar, c_void, size_t};
use log::error;

use crate::stream::Tag;
use crate::stream_tags::StreamTags;
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
            base: buf.cast::<c_uchar>(),
            len,
        })
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: This is what we mmapped.
        let rc = unsafe { libc::munmap(self.base.cast::<c_void>(), self.len) };
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
        fn set_len(f: &std::fs::File, size: usize) -> Result<()> {
            f.set_len(size as u64)
                .map_err(|e| Error::wrap(e, format!("circular buffer temp file set_len({size})")))
        }

        let size_x2 = size * 2;
        // Annotating the temp dir directory may help in case of not enough
        // space, permissions, etc.
        let f = tempfile::tempfile().map_err(|e| Error::file_io(e, std::env::temp_dir()))?;
        set_len(&f, size_x2)?;
        // Map first half.
        let mut map = Map::new(&f, size_x2)?;

        // Remap second half to be same as the first.
        // Be very careful with the order, here.
        let second = (map.base as libc::uintptr_t + size as libc::uintptr_t) as *mut c_void;
        let map2 = Map::with_addr(&f, size, second)?;

        // First map is now just `size`.
        map.len = size;

        // Shrink file.
        set_len(&f, size)?;

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

    /// Borrow a mutable window of initialized elements.
    ///
    /// # Safety
    ///
    /// The range must contain valid `T`s and must not alias another live
    /// reference, including through the second mapping of the same pages.
    #[allow(clippy::mut_from_ref)]
    #[must_use]
    unsafe fn full_buffer<T>(&self, start: usize, end: usize) -> &mut [T] {
        let ez = std::mem::size_of::<T>();
        assert!(ez != 0 && self.len.is_multiple_of(ez));
        assert!(start <= end);
        assert!(
            end - start <= self.len / ez / 2,
            "requested {start} to {end} ({} entries) of {} but len is {}",
            end - start,
            ez,
            self.len
        );
        assert!(end <= self.len / ez);
        // SAFETY: the caller guarantees initialization and exclusive access;
        // the bounds above keep the slice inside the mapping.
        unsafe { std::slice::from_raw_parts_mut(self.map.base.cast::<T>().add(start), end - start) }
    }
}

// SAFETY: Circ are just metadata around normal process-local memory.
unsafe impl Send for Circ {}
// SAFETY: Circ are just metadata around normal process-local memory.
unsafe impl Sync for Circ {}

#[derive(Debug)]
struct BufferState<T> {
    rpos: usize,        // In samples.
    wpos: usize,        // In samples.
    used: usize,        // In samples.
    circ_len: usize,    // In bytes.
    member_size: usize, // In bytes.
    tags: StreamTags,
    reader_active: bool,
    writer_active: bool,
    // The write endpoint is gone; a checked-out window may still commit.
    producer_closed: bool,
    observers: crate::stream_observer::Observers<T>,
}

impl<T> BufferState<T> {
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

/// `BufferReader` is an RAII'd fixed window read slice with some helper functions.
pub struct BufferReader<T> {
    parent: Arc<Buffer<T>>,
    start: usize,
    end: usize,
    // consume/produce already released the checkout under the state lock.
    released: bool,
}

impl<T: Copy> BufferReader<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> Self {
        Self {
            parent,
            start,
            end,
            released: false,
        }
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
    pub fn consume(mut self, n: usize) {
        assert!(n <= self.len(), "consume exceeds checked-out read window");
        self.parent.consume(n);
        self.released = true;
    }

    /// len convenience function.
    #[must_use]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// `is_empty` convenience function.
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

impl<T> Drop for BufferReader<T> {
    fn drop(&mut self) {
        if !self.released
            && let Ok(mut state) = self.parent.state.lock.lock()
        {
            state.reader_active = false;
        }
    }
}

/// `BufferWriter` is an RAII fixed window slice with some helper functions.
pub struct BufferWriter<T> {
    parent: Arc<Buffer<T>>,
    start: usize,
    end: usize,
    // consume/produce already released the checkout under the state lock.
    released: bool,
}

impl<T: Copy> BufferWriter<T> {
    #[must_use]
    fn new(parent: Arc<Buffer<T>>, start: usize, end: usize) -> BufferWriter<T> {
        assert!(end >= start);
        Self {
            parent,
            start,
            end,
            released: false,
        }
    }

    /// Return the slice to write to.
    #[must_use]
    pub fn slice(&mut self) -> &mut [T] {
        // SAFETY: the sole writer owns the free range, disjoint from the
        // reader's unconsumed samples. All elements were initialized in new().
        unsafe { self.parent.circ.full_buffer::<T>(self.start, self.end) }
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

    /// Commit `n` samples and tags with positions relative to this window.
    ///
    /// Borrowed tags (such as `&[Tag]` or `&Vec<Tag>`) are cloned into stream
    /// storage. Owned tags (`Vec<Tag>`) are moved without cloning their keys
    /// or values.
    ///
    /// Tags are sorted by position, preserving the order of tags at the same
    /// position. Positions must be less than `n`; use no tags when `n` is zero.
    ///
    /// # Panics
    ///
    /// Panics if `n` exceeds the write window, or if `n` is nonzero and a tag
    /// is outside the produced samples. Zero-sample writes ignore tags and
    /// log an error if any were supplied.
    pub fn produce<'a>(mut self, n: usize, tags: impl Into<Cow<'a, [Tag]>>) {
        assert!(n <= self.len(), "produce exceeds checked-out write window");
        self.parent.produce(n, tags.into());
        self.released = true;
    }

    /// len convenience function.
    #[must_use]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// `is_empty` convenience function.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

impl<T> Drop for BufferWriter<T> {
    fn drop(&mut self) {
        if !self.released {
            let mut state = self
                .parent
                .state
                .lock
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.writer_active = false;
            if state.producer_closed {
                self.parent.state.metadata.set_ended();
                state.observers.end();
            }
        }
    }
}

#[derive(Debug)]
struct BufferInner<T> {
    // Stream registry has a weak pointer to metadata. Other than that it's not
    // shared.
    metadata: Arc<crate::stream_metadata::Metadata>,
    lock: Mutex<BufferState<T>>,
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
    // Typed observer controls share state without retaining the sample mapping.
    state: Arc<BufferInner<T>>,
    _control: Option<Arc<crate::stream_observer::TypedControl<T>>>,
    circ: Circ,
    member_size: usize,
    dummy: std::marker::PhantomData<T>,
}

impl<T> Buffer<T> {
    /// Create a new Buffer.
    ///
    /// Size is in bytes and must be a positive multiple of the page size
    /// and element size. Elements must be nonzero sized. The write storage
    /// starts with valid default values; zero bits need not be valid for `T`.
    pub fn new(size: usize) -> Result<Self>
    where
        T: Copy + Default + Send + 'static,
    {
        const {
            assert!(std::mem::size_of::<T>() != 0);
        }

        let member_size = std::mem::size_of::<T>();
        if size == 0 || size > isize::MAX as usize / 2 {
            return Err(Error::msg("unsupported circular buffer size"));
        }
        if !size.is_multiple_of(member_size) {
            return Err(Error::msg("buffer size must be a multiple of element size"));
        }
        // SAFETY: sysconf takes no pointers and reads the system page size.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 || !size.is_multiple_of(page_size as usize) {
            return Err(Error::msg("buffer size must be a multiple of page size"));
        }
        let circ = Circ::new(size)?;
        if !(circ.map.base as usize).is_multiple_of(std::mem::align_of::<T>()) {
            return Err(Error::msg("element alignment exceeds mapping alignment"));
        }
        // Initialize through raw writes before creating any typed reference.
        // Only the first mapping needs initialization: the second aliases it.
        for index in 0..size / member_size {
            // SAFETY: the mapping is aligned, large enough, and exclusively
            // owned here. Copy elements need no destructor on overwrite/unmap.
            unsafe { circ.map.base.cast::<T>().add(index).write(T::default()) };
        }
        let id = crate::NEXT_STREAM_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let state = Arc::new(BufferInner {
            metadata: crate::stream_metadata::Metadata::new::<T>(id, false),
            lock: Mutex::new(BufferState {
                rpos: 0,
                wpos: 0,
                used: 0,
                circ_len: size,
                member_size: std::mem::size_of::<T>(),
                tags: StreamTags::default(),
                reader_active: false,
                writer_active: false,
                producer_closed: false,
                observers: Default::default(),
            }),
            cv: Condvar::new(),
            #[cfg(feature = "async")]
            acvr: tokio::sync::Notify::new(),
            #[cfg(feature = "async")]
            acvw: tokio::sync::Notify::new(),
        });
        let control = (state.metadata.kind != crate::stream_observer::Kind::Unsupported)
            .then(|| Arc::new(crate::stream_observer::TypedControl(state.clone())));
        if let Some(control) = &control {
            crate::stream_metadata::Metadata::register(id, control.clone());
        }
        Ok(Self {
            id,
            state,
            _control: control,
            member_size: std::mem::size_of::<T>(),
            circ,
            dummy: std::marker::PhantomData,
        })
    }

    #[must_use]
    pub(crate) fn metadata(&self) -> &crate::stream_metadata::Metadata {
        &self.state.metadata
    }

    pub(crate) fn close_writer(&self) {
        let mut state = self.state.lock.lock().unwrap_or_else(|p| p.into_inner());
        state.producer_closed = true;
        if !state.writer_active {
            self.state.metadata.set_ended();
            state.observers.end();
        }
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

    /// Wait for ability to write at least `need` elements.
    ///
    /// May return early with a lower value, to not block.
    ///
    /// Returns free elements.
    #[must_use]
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

    /// Wait for ability to write at least `need` samples.
    ///
    /// May return early with a lower value, to not block.
    ///
    /// Returns free samples.
    #[cfg(feature = "async")]
    pub async fn wait_for_write_async(&self, need: usize) -> usize {
        tokio::time::timeout(ASYNC_SLEEP_TIME, async {
            loop {
                {
                    let n = self.state.lock.lock().unwrap().free();
                    if n >= need {
                        return n;
                    }
                }
                self.state.acvw.notified().await;
            }
        })
        .await
        .unwrap_or_else(|_| self.state.lock.lock().unwrap().free())
    }

    /// Wait for ability to read `need` samples.
    ///
    /// May return early with a lower value, to not block.
    ///
    /// Returns number of available samples.
    #[must_use]
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

    /// Wait for ability to read `need` samples.
    ///
    /// May return early with a lower value, to not block.
    ///
    /// Returns number of available samples.
    #[cfg(feature = "async")]
    pub async fn wait_for_read_async(&self, need: usize) -> usize {
        tokio::time::timeout(ASYNC_SLEEP_TIME, async {
            loop {
                {
                    let n = self.state.lock.lock().unwrap().used;
                    if n >= need {
                        return n;
                    }
                }
                self.state.acvr.notified().await;
            }
        })
        .await
        .unwrap_or_else(|_| self.state.lock.lock().unwrap().used)
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
    pub(in crate::nowasm::circular_buffer) fn consume(&self, n: usize) {
        let mut s = self.state.lock.lock().unwrap();
        if n == 0 {
            s.reader_active = false;
            return;
        }
        assert!(
            n <= s.used,
            "trying to consume {}, but only have {}",
            n,
            s.used
        );
        let newpos = (s.rpos + n) % s.capacity();

        s.tags.consume(n);
        s.rpos = newpos;
        s.used -= n;
        s.reader_active = false;
        self.state.cv.notify_all();
        #[cfg(feature = "async")]
        self.state.acvw.notify_one();
    }

    /// Produce samples (commit writes).
    ///
    /// Will only be called from the write buffer.
    pub(in crate::nowasm::circular_buffer) fn produce(&self, n: usize, tags: Cow<'_, [Tag]>) {
        let mut s = self.state.lock.lock().unwrap();
        if n == 0 {
            s.writer_active = false;
            if s.producer_closed {
                self.state.metadata.set_ended();
                s.observers.end();
            }
            if !tags.is_empty() {
                error!("produce() called on a stream with 0 entries, but non-empty tags: {tags:?}");
            }
            return;
        }
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
        if !s.observers.0.is_empty() {
            assert!(
                tags.iter().all(|t| t.pos() < n),
                "tag outside produced samples"
            );
            let position = s.tags.position();
            let samples = self.slice(s.wpos, s.wpos + n);
            s.observers.samples(samples, &tags, position);
        }
        s.tags.produce(n, tags);
        s.wpos = (s.wpos + n) % s.capacity();
        s.used += n;
        s.writer_active = false;
        if s.producer_closed {
            self.state.metadata.set_ended();
            s.observers.end();
        }
        self.state.cv.notify_all();
        #[cfg(feature = "async")]
        self.state.acvr.notify_waiters();
    }

    #[must_use]
    fn slice(&self, start: usize, end: usize) -> &[T] {
        // SAFETY: the reader owns the only read window. Only unconsumed
        // samples are exposed, and the writer can access only the free range.
        unsafe {
            std::slice::from_raw_parts(self.circ.map.base.cast::<T>().add(start), end - start)
        }
    }

    /// Get the read slice.
    ///
    /// TODO: no need for Result in API.
    pub fn read_buf(self: Arc<Self>) -> Result<(BufferReader<T>, Vec<Tag>)> {
        let mut s = self.state.lock.lock().unwrap();
        if s.reader_active {
            return Err(Error::msg(
                "read_buf() called with an outstanding read buffer",
            ));
        }
        let (start, end) = s.read_range();
        let tags = s.tags.read();
        s.reader_active = true;
        drop(s);
        Ok((BufferReader::new(self, start, end), tags))
    }

    /// Get the write slice.
    pub fn write_buf(self: Arc<Self>) -> Result<BufferWriter<T>> {
        let mut s = self.state.lock.lock().unwrap();
        if s.writer_active {
            return Err(Error::msg(
                "write_buf() called with an outstanding write buffer",
            ));
        }
        let (start, end) = s.write_range();
        s.writer_active = true;
        drop(s);
        Ok(BufferWriter::new(self, start, end))
    }
}

impl<T> crate::stream_observer::Control<T> for BufferInner<T> {
    fn with_state(
        &self,
        action: &mut dyn FnMut(&mut crate::stream_observer::Observers<T>, u64, bool),
    ) {
        let mut s = self.lock.lock().unwrap();
        let position = s.tags.position();
        let ended = s.producer_closed && !s.writer_active;
        action(&mut s.observers, position, ended);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn initializes_types_that_cannot_contain_zero_bits() -> Result<()> {
        #[derive(Clone, Copy, Debug)]
        struct Nonzero(std::num::NonZeroU8);
        impl Default for Nonzero {
            fn default() -> Self {
                Self(std::num::NonZeroU8::new(7).unwrap())
            }
        }
        let buffer = Arc::new(Buffer::<Nonzero>::new(4096)?);
        let mut writer = buffer.clone().write_buf()?;
        assert!(writer.slice().iter().all(|sample| sample.0.get() == 7));
        writer.produce(4096, &[]);
        let (reader, _) = buffer.read_buf()?;
        assert!(reader.iter().all(|sample| sample.0.get() == 7));
        Ok(())
    }

    #[test]
    fn rejects_invalid_buffer_layouts() {
        assert!(Buffer::<u8>::new(0).is_err());
        assert!(Buffer::<u8>::new(4095).is_err());
        assert!(Buffer::<[u8; 3]>::new(4096).is_err());
    }

    use super::*;

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_read_wait_reports_buffered_samples() -> Result<()> {
        let buffer = Arc::new(Buffer::<u8>::new(4096)?);
        let mut writer = buffer.clone().write_buf()?;
        writer.fill_from_slice(&[1, 2, 3]);
        writer.produce(3, &[]);
        // notify_waiters above leaves no permit: exercise the timeout path.
        assert_eq!(buffer.wait_for_read_async(2).await, 3);
        buffer.state.acvr.notify_one();
        assert_eq!(buffer.wait_for_read_async(2).await, 3);
        let input = crate::stream::ReadStream::from_slice(&[1u8, 2, 3]);
        assert_eq!(
            input.wait_for_read_async(2).await,
            crate::stream::WaitOutcome::Ready
        );
        Ok(())
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_write_wait_reports_free_samples() -> Result<()> {
        let buffer = Arc::new(Buffer::<u8>::new(4096)?);
        let mut writer = buffer.clone().write_buf()?;
        writer.fill_from_slice(&[1, 2, 3]);
        writer.produce(3, &[]);
        assert_eq!(buffer.wait_for_write_async(2).await, 4093);
        buffer.clone().read_buf()?.0.consume(1);
        assert_eq!(buffer.wait_for_write_async(2).await, 4094);
        Ok(())
    }
    use crate::Float;
    use crate::stream::TagValue;

    #[test]
    fn map_empty_file() -> Result<()> {
        let f = std::fs::File::open("/dev/null")?;
        assert!(Map::new(&f, 1024).is_err());
        Ok(())
    }

    #[test]
    fn circ_reqlen() -> Result<()> {
        let circ = Circ::new(4096)?;
        assert_eq!(circ.total_size(), 4096);
        // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
        // are retained, and invalid bounds panic before borrowing.
        assert!(unsafe { circ.full_buffer::<u32>(0, 0) }.is_empty());
        // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
        // are retained, and invalid bounds panic before borrowing.
        assert_eq!(unsafe { circ.full_buffer::<u32>(0, 1) }.len(), 1);
        // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
        // are retained, and invalid bounds panic before borrowing.
        assert_eq!(unsafe { circ.full_buffer::<u32>(0, 1024) }.len(), 1024);
        // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
        // are retained, and invalid bounds panic before borrowing.
        assert_eq!(unsafe { circ.full_buffer::<u32>(1000, 1200) }.len(), 200);
        // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
        // are retained, and invalid bounds panic before borrowing.
        assert_eq!(unsafe { circ.full_buffer::<u32>(2040, 2048) }.len(), 8);
        Ok(())
    }

    // Can't even calculate the size of the double buffer.
    #[test]
    #[should_panic]
    fn circ_reqlen_too_big() {
        let _ = Circ::new(usize::MAX);
    }

    // We can calculate it, but we either can't create that big a tempfile, or
    // we can't map it.
    #[test]
    fn circ_reqlen_too_big_half() {
        assert!(Circ::new(usize::MAX >> 1).is_err());
    }

    #[test]
    #[should_panic]
    fn circ_reqlen_too_big_beginning() {
        if let Ok(circ) = Circ::new(4096) {
            // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
            // are retained, and invalid bounds panic before borrowing.
            let _ = unsafe { circ.full_buffer::<u32>(0, 1025) };
        }
    }

    #[test]
    #[should_panic]
    fn circ_reqlen_too_big_middle() {
        if let Ok(circ) = Circ::new(4096) {
            // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
            // are retained, and invalid bounds panic before borrowing.
            let _ = unsafe { circ.full_buffer::<u32>(10, 1024 + 11) };
        }
    }

    #[test]
    #[should_panic]
    fn circ_past_end() {
        if let Ok(circ) = Circ::new(4096) {
            // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
            // are retained, and invalid bounds panic before borrowing.
            let _ = unsafe { circ.full_buffer::<u32>(2040, 2049) };
        }
    }

    #[test]
    fn circ_circular() -> Result<()> {
        let circ = Circ::new(4096)?;
        assert_eq!(circ.total_size(), 4096);
        let first = {
            // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
            // are retained, and invalid bounds panic before borrowing.
            let buf = unsafe { circ.full_buffer::<u32>(0, 4) };
            assert_eq!(buf, [0, 0, 0, 0]);
            buf.copy_from_slice(&[3, 2, 1, 42]);
            buf.as_ptr()
        };
        // SAFETY: fresh mappings contain valid zeroed u32s; no aliases
        // are retained, and invalid bounds panic before borrowing.
        let buf2 = unsafe { circ.full_buffer::<u32>(1024, 1028) };
        assert_eq!(buf2, [3, 2, 1, 42]);
        assert_ne!(first, buf2.as_ptr());
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
