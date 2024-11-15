//! Test implementation of circular buffers.
//! Full of unsafe. Full of ugly code.

use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use libc::{c_uchar, c_void, size_t};
use libc::{MAP_FAILED, MAP_FIXED, MAP_SHARED, PROT_READ, PROT_WRITE};

use crate::stream::{Tag, TagPos};
use crate::Error;

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
        let buf = unsafe { libc::mmap(ptr, len as size_t, PROT_READ | PROT_WRITE, flags, fd, 0) };
        if buf == MAP_FAILED {
            let e = errno::errno();
            return Err(Error::new(&format!(
                "mmap(){}: {e}",
                if ptr.is_null() {
                    ""
                } else {
                    " at fixed address"
                }
            ))
            .into());
        }
        assert!(!buf.is_null());
        if !ptr.is_null() && ptr != buf {
            let rc = unsafe { libc::munmap(buf, len as size_t) };
            if rc != 0 {
                let e = errno::errno();
                panic!("Failed to unmap buffer just mapped in the failure path: {e}");
            }
            return Err(Error::new("mmap() allocated in the wrong place").into());
        }
        Ok(Self {
            base: buf as *mut c_uchar,
            len,
        })
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        let rc = unsafe { libc::munmap(self.base as *mut c_void, self.len) };
        if rc != 0 {
            let e = errno::errno();
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
        let f = tempfile::tempfile()?;
        f.set_len(size as u64)?;
        let len2 = size * 2;

        // Map first half.
        let mut map = Map::new(&f, len2)?;

        // Remap second half to be same as the first.
        // Be very careful with the order, here.
        let second = (map.base as libc::uintptr_t + size as libc::uintptr_t) as *mut c_void;
        let map2 = Map::with_addr(&f, size, second)?;
        map.len = size;

        Ok(Self {
            len: len2,
            map,
            _map2: map2,
        })
    }

    /// Return length of buffer, *before* the double mapping, in bytes.
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
    #[allow(clippy::mut_from_ref)]
    fn full_buffer<T>(&self, start: usize, end: usize) -> &mut [T] {
        assert!(self.len % std::mem::size_of::<T>() == 0);
        let buf = unsafe {
            std::slice::from_raw_parts_mut(
                self.map.base as *mut T,
                self.len / std::mem::size_of::<T>(),
            )
        };
        &mut buf[start..end]
    }
}

unsafe impl Send for Circ {}
unsafe impl Sync for Circ {}

#[derive(Debug)]
struct BufferState {
    rpos: usize,        // In samples.
    wpos: usize,        // In samples.
    used: usize,        // In samples.
    circ_len: usize,    // In bytes.
    member_size: usize, // In bytes.
    read_borrow: bool,
    write_borrow: bool,
    tags: BTreeMap<TagPos, Vec<Tag>>,
}

impl BufferState {
    // Return write range, in samples.
    fn write_range(&self) -> (usize, usize) {
        //eprintln!("Write range: {} {}", self.rpos, self.wpos);
        (self.wpos, self.wpos + self.free())
    }

    // Read range, in samples
    fn read_range(&self) -> (usize, usize) {
        (self.rpos, self.rpos + self.used)
    }

    // In samples.
    fn capacity(&self) -> usize {
        self.circ_len / self.member_size
    }

    // Write capacity, in samples.
    fn write_capacity(&self) -> usize {
        let (a, b) = self.write_range();
        b - a
    }

    // Free space, in samples
    fn free(&self) -> usize {
        self.capacity() - self.used
    }
}

/// BufferReader is an RAII'd read slice with some helper functions.
pub struct BufferReader<'a, T: Copy> {
    slice: &'a [T],
    parent: &'a Buffer<T>,
}

impl<'a, T: Copy> BufferReader<'a, T> {
    fn new(slice: &'a [T], parent: &'a Buffer<T>) -> BufferReader<'a, T> {
        Self { slice, parent }
    }

    /// Return slice to read from.
    pub fn slice(&self) -> &[T] {
        self.slice
    }

    /// Helper function to iterate over input instead.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.slice.iter()
    }

    /// We're done with the buffer. Consume `n` samples.
    pub fn consume(self, n: usize) {
        self.parent.consume(n);
    }

    /// len convenience function.
    pub fn len(&self) -> usize {
        self.slice.len()
    }

    /// is_empty convenience function.
    pub fn is_empty(&self) -> bool {
        self.slice.is_empty()
    }
}

impl<T: Copy> std::ops::Index<usize> for BufferReader<'_, T> {
    type Output = T;

    fn index(&self, n: usize) -> &Self::Output {
        &self.slice[n]
    }
}

impl<T: Copy> Drop for BufferReader<'_, T> {
    fn drop(&mut self) {
        self.parent.return_read_buf();
    }
}

/// BufferWriter is an RAII slice with some helper functions.
pub struct BufferWriter<'a, T: Copy> {
    slice: &'a mut [T],
    parent: &'a Buffer<T>,
}

impl<'a, T: Copy> BufferWriter<'a, T> {
    fn new(slice: &'a mut [T], parent: &'a Buffer<T>) -> BufferWriter<'a, T> {
        Self { slice, parent }
    }

    /// Return the slice to write to.
    pub fn slice(&mut self) -> &mut [T] {
        self.slice
    }

    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_iter(&mut self, src: impl IntoIterator<Item = T>) {
        for (place, item) in self.slice.iter_mut().zip(src) {
            *place = item;
        }
    }

    /// Shortcut to save typing for the common operation of copying
    /// from an iterator.
    pub fn fill_from_slice(&mut self, src: &[T]) {
        self.slice[..src.len()].copy_from_slice(src);
    }

    /// Having written into the write buffer, now tell the buffer
    /// we're done. Also here are the tags, with positions relative to
    /// start of buffer.
    pub fn produce(self, n: usize, tags: &[Tag]) {
        self.parent.produce(n, tags);
    }

    /// len convenience function.
    pub fn len(&self) -> usize {
        self.slice.len()
    }

    /// is_empty convenience function.
    pub fn is_empty(&self) -> bool {
        self.slice.is_empty()
    }
}

impl<T: Copy> Drop for BufferWriter<'_, T> {
    fn drop(&mut self) {
        self.parent.return_write_buf();
    }
}

/// Type aware buffer.
#[derive(Debug)]
pub struct Buffer<T> {
    state: Arc<Mutex<BufferState>>,
    circ: Circ,
    member_size: usize,
    dummy: std::marker::PhantomData<T>,
}

impl<T> Buffer<T> {
    /// Create a new Buffer.
    pub fn new(size: usize) -> Result<Self> {
        Ok(Self {
            state: Arc::new(Mutex::new(BufferState {
                read_borrow: false,
                write_borrow: false,
                rpos: 0,
                wpos: 0,
                used: 0,
                circ_len: size,
                member_size: std::mem::size_of::<T>(),
                tags: BTreeMap::new(),
            })),
            member_size: std::mem::size_of::<T>(),
            circ: Circ::new(size)?,
            dummy: std::marker::PhantomData,
        })
    }

    /// Return length of buffer, ignoring how much is in use, and the
    /// double buffer.
    pub fn total_size(&self) -> usize {
        self.circ.total_size() / self.member_size
    }
}

impl<T: Copy> Buffer<T> {
    /// Consume samples from input buffer.
    ///
    /// Will only be called from the read buffer.
    pub(in crate::circular_buffer) fn consume(&self, n: usize) {
        let mut s = self.state.lock().unwrap();
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
    }

    /// Produce samples (commit writes).
    ///
    /// Will only be called from the write buffer.
    pub(in crate::circular_buffer) fn produce(&self, n: usize, tags: &[Tag]) {
        let mut s = self.state.lock().unwrap();
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
            let tag = Tag::new(pos, tag.key().into(), tag.val().clone());
            s.tags.entry(pos).or_default().push(tag);
        }
        s.wpos = (s.wpos + n) % s.capacity();
        s.used += n;
    }

    /// Will only be called from the read buffer, as it gets destroyed.
    pub(in crate::circular_buffer) fn return_read_buf(&self) {
        let mut s = self.state.lock().unwrap();
        assert!(s.read_borrow);
        s.read_borrow = false;
    }

    /// Will only be called from the write buffer, as it gets destroyed.
    pub(in crate::circular_buffer) fn return_write_buf(&self) {
        let mut s = self.state.lock().unwrap();
        assert!(s.write_borrow);
        s.write_borrow = false;
    }

    /// Get the read slice.
    pub fn read_buf(&self) -> Result<(BufferReader<T>, Vec<Tag>)> {
        let mut s = self.state.lock().unwrap();
        if s.read_borrow {
            return Err(Error::new("read buf already borrowed").into());
        }
        s.read_borrow = true;
        let (start, end) = s.read_range();
        let buf = self.circ.full_buffer::<T>(start, end);
        let mut tags = Vec::new();

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
                    tag.key().into(),
                    tag.val().clone(),
                ));
            }
        }
        tags.sort_by_key(|a| a.pos());
        Ok((
            BufferReader::new(unsafe { std::mem::transmute::<&mut [T], &[T]>(buf) }, self),
            tags,
        ))
    }

    /// Get the write slice.
    pub fn write_buf(&self) -> Result<BufferWriter<T>> {
        let mut s = self.state.lock().unwrap();
        if s.write_borrow {
            return Err(Error::new("write buf already borrowed").into());
        }
        s.write_borrow = true;
        let (start, end) = s.write_range();
        let buf = self.circ.full_buffer::<T>(start, end);
        Ok(BufferWriter::new(
            unsafe { std::mem::transmute::<&mut [T], &mut [T]>(buf) },
            self,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::TagValue;
    use crate::Float;

    #[test]
    pub fn test_no_double() -> Result<()> {
        let b = Arc::new(Buffer::<u8>::new(4096)?);
        {
            let _i1 = b.read_buf()?;
            assert!(b.read_buf().is_err());
        }
        let _i2 = b.read_buf()?;
        {
            let _w1 = b.write_buf()?;
            assert!(b.write_buf().is_err());
        }
        let _w2 = b.write_buf()?;
        Ok(())
    }

    #[test]
    pub fn test_typical() -> Result<()> {
        let b: Buffer<u8> = Buffer::new(4096)?;

        // Initial.
        assert!(b.read_buf()?.0.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);

        // Write a byte.
        {
            let mut buf = b.write_buf()?;
            buf.slice()[0] = 123;
            buf.produce(1, &[Tag::new(0, "start".into(), TagValue::Bool(true))]);
            assert_eq!(b.read_buf()?.0.slice(), vec![123]);
            assert_eq!(
                b.read_buf()?.1,
                vec![Tag::new(0, "start".into(), TagValue::Bool(true))]
            );
            assert_eq!(b.write_buf()?.len(), 4095);
        }

        // Consume the byte.
        b.consume(1);
        assert!(b.read_buf()?.0.is_empty());
        assert!(b.read_buf()?.1.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);

        // Write towards the end bytes.
        {
            let n = 4000;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = (i & 0xff) as u8;
            }
            wb.produce(
                n,
                &[Tag::new(1, "foo".into(), TagValue::String("bar".into()))],
            );
            let (rb, rt) = b.read_buf()?;
            assert_eq!(rb.len(), n);
            for i in 0..n {
                assert_eq!(rb.slice()[i], (i & 0xff) as u8);
            }
            assert_eq!(
                rt,
                vec![Tag::new(1, "foo".into(), TagValue::String("bar".into()))]
            );
            assert_eq!(b.write_buf()?.len(), 4096 - n);
        }
        b.consume(4000);

        // Write 100 bytes.
        {
            let n = 100;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = ((n - i) & 0xff) as u8;
            }
            wb.produce(
                n,
                &[
                    Tag::new(0, "first".into(), TagValue::Bool(true)),
                    Tag::new(99, "last".into(), TagValue::Bool(false)),
                ],
            );
            let (rb, rt) = b.read_buf()?;
            assert_eq!(rb.len(), n);
            for i in 0..n {
                assert_eq!(rb.slice()[i], ((n - i) & 0xff) as u8);
            }
            assert_eq!(
                rt,
                vec![
                    Tag::new(0, "first".into(), TagValue::Bool(true)),
                    Tag::new(99, "last".into(), TagValue::Bool(false))
                ]
            );
            drop(rb);
            assert_eq!(b.read_buf()?.0.len(), 100);
            assert_eq!(b.write_buf()?.len(), 3996);
        }

        // Clear it.
        {
            let (rb, _) = b.read_buf()?;
            let n = rb.len();
            rb.consume(n);
            assert_eq!(b.read_buf()?.0.len(), 0);
            assert!(b.read_buf()?.1.is_empty());
            assert_eq!(b.write_buf()?.len(), 4096);
        }
        Ok(())
    }

    #[test]
    pub fn test_two_writes() -> Result<()> {
        let b: Buffer<u8> = Buffer::new(4096)?;

        // Write 10 bytes.
        {
            let mut buf = b.write_buf()?;
            buf.slice()[1] = 123;
            buf.produce(10, &[Tag::new(1, "first".into(), TagValue::Bool(true))]);
            assert_eq!(
                b.read_buf()?.0.slice(),
                vec![0, 123, 0, 0, 0, 0, 0, 0, 0, 0]
            );
            assert_eq!(
                b.read_buf()?.1,
                vec![Tag::new(1, "first".into(), TagValue::Bool(true))]
            );
            assert_eq!(b.write_buf()?.len(), 4086);
        }

        // Write 5 more bytes.
        {
            let mut buf = b.write_buf()?;
            buf.slice()[2] = 42;
            buf.produce(5, &[Tag::new(2, "second".into(), TagValue::Bool(false))]);
            assert_eq!(
                b.read_buf()?.0.slice(),
                vec![0, 123, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 42, 0, 0]
            );
            assert_eq!(
                b.read_buf()?.1,
                vec![
                    Tag::new(1, "first".into(), TagValue::Bool(true)),
                    Tag::new(12, "second".into(), TagValue::Bool(false))
                ]
            );
            assert_eq!(b.write_buf()?.len(), 4081);
        }

        // Consume the byte.
        b.consume(15);
        assert!(b.read_buf()?.0.is_empty());
        assert!(b.read_buf()?.1.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);
        Ok(())
    }

    #[test]
    pub fn exact_overflow() -> Result<()> {
        let b: Buffer<u8> = Buffer::new(4096)?;

        // Initial.
        assert!(b.read_buf()?.0.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);

        // Full.
        b.write_buf()?.produce(4096, &[]);
        assert_eq!(b.read_buf()?.0.len(), 4096);
        assert_eq!(b.write_buf()?.len(), 0);

        // Empty again.
        b.read_buf()?.0.consume(4096);
        assert!(b.read_buf()?.0.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);
        Ok(())
    }

    #[test]
    pub fn test_float() -> Result<()> {
        let b: Buffer<Float> = Buffer::new(4096)?;

        // Initial.
        assert!(b.read_buf()?.0.is_empty());
        assert_eq!(b.write_buf()?.len(), 1024);

        // Write a sample.
        {
            let mut wb = b.write_buf()?;
            wb.slice()[0] = 123.321;
            wb.produce(1, &[]);
        }
        assert_eq!(b.read_buf()?.0.slice(), vec![123.321]);
        assert_eq!(b.write_buf()?.len(), 1023);

        // Consume the sample.
        b.read_buf()?.0.consume(1);
        assert!(b.read_buf()?.0.is_empty());
        assert_eq!(b.write_buf()?.len(), 1024);

        // Write towards the end bytes.
        {
            let n = 1000;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = i as Float;
            }
            wb.produce(n, &[]);
            assert_eq!(b.read_buf()?.0.len(), n);
            for i in 0..n {
                assert_eq!(b.read_buf()?.0.slice()[i], i as Float);
            }
            assert_eq!(b.write_buf()?.len(), 24);
        }
        b.read_buf()?.0.consume(1000);

        // Write 100 bytes.
        {
            let n = 100;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = (n - i) as Float;
            }
            wb.produce(n, &[]);
            assert_eq!(b.read_buf()?.0.len(), n);
            for i in 0..n {
                assert_eq!(b.read_buf()?.0.slice()[i], (n - i) as Float);
            }
        }
        assert_eq!(b.read_buf()?.0.len(), 100);
        assert_eq!(b.write_buf()?.len(), 1024 - 100);
        Ok(())
    }
}
/* vim: textwidth=80
 */
