//! Test implementation of circular buffers.
//! Full of unsafe. Full of ugly code.
//!
//! TODO:
//! * Tag support.
//! * Rewrite all blocks for this API.

use std::collections::{BTreeMap, VecDeque};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use libc::{c_int, c_uchar, c_void, off_t, size_t};
use libc::{MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};

use crate::stream::{Tag, TagPos};
use crate::Error;

extern "C" {
    fn mmap(
        addr: *const c_void,
        len: size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: off_t,
    ) -> *mut c_void;
    fn munmap(addr: *const c_void, length: size_t) -> c_int;
}

/// Circular buffer dealing in bytes.
#[derive(Debug)]
pub struct Circ {
    buf: *mut c_uchar,
    len: usize,
}

impl Circ {
    /// Create a new circular buffer.
    ///
    /// TODO:
    /// * don't leak memory on error.
    /// * release memory on drop.
    pub fn new() -> Result<Self> {
        let len = 4096usize;
        let len2 = len * 2;
        let f = tempfile::tempfile()?;
        f.set_len(len2 as u64)?;
        let fd = f.as_raw_fd();

        // Map first.
        let buf = unsafe {
            let buf = mmap(
                std::ptr::null::<c_void>(),
                len2 as size_t,
                PROT_READ | PROT_WRITE,
                MAP_SHARED, // flags
                fd,         // fd
                0,          // offset
            );
            if buf == MAP_FAILED {
                panic!();
            }
            buf as *mut c_uchar
        };
        let second = (buf as libc::uintptr_t + len as libc::uintptr_t) as *const c_void;
        // Unmap second half.
        unsafe {
            let rc = munmap(second, len);
            if rc != 0 {
                panic!();
            }
        }
        // Map second half.
        unsafe {
            let buf = mmap(
                second as *const c_void,
                len as size_t,
                PROT_READ | PROT_WRITE,
                MAP_SHARED, // flags
                fd,         // fd
                0,          // offset
            );
            if buf == MAP_FAILED {
                panic!();
            }
            assert_eq!(buf as *const c_void, second);
        };
        Ok(Self { len: len2, buf })
    }
    fn full_buffer<T>(&self) -> &mut [T] {
        assert!(self.len % std::mem::size_of::<T>() == 0);
        unsafe {
            std::slice::from_raw_parts_mut(self.buf as *mut T, self.len / std::mem::size_of::<T>())
        }
    }
    fn len(&self) -> usize {
        self.len / 2
    }
}

unsafe impl Send for Circ {}
unsafe impl Sync for Circ {}

#[derive(Debug)]
struct BufferState<T> {
    rpos: usize,        // In samples.
    wpos: usize,        // In samples.
    used: usize,        // In samples.
    circ_len: usize,    // In bytes.
    member_size: usize, // In bytes.
    read_borrow: bool,
    write_borrow: bool,
    noncopy: VecDeque<T>,
    tags: BTreeMap<TagPos, Vec<Tag>>,
}

impl<T> BufferState<T> {
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

pub struct BufferReader<'a, T: Copy> {
    slice: &'a [T],
    parent: &'a Buffer<T>,
}

impl<'a, T: Copy> BufferReader<'a, T> {
    fn new(slice: &'a [T], parent: &'a Buffer<T>) -> BufferReader<'a, T> {
        Self { slice, parent }
    }
    pub fn slice(&self) -> &[T] {
        self.slice
    }
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.slice.iter()
    }
    pub fn consume(self, n: usize) {
        self.parent.consume(n);
    }
    pub fn len(&self) -> usize {
        self.slice.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slice.is_empty()
    }
}

impl<T: Copy> Drop for BufferReader<'_, T> {
    fn drop(&mut self) {
        self.parent.return_read_buf();
    }
}

pub struct BufferWriter<'a, T: Copy> {
    slice: &'a mut [T],
    parent: &'a Buffer<T>,
}

impl<'a, T: Copy> BufferWriter<'a, T> {
    fn new(slice: &'a mut [T], parent: &'a Buffer<T>) -> BufferWriter<'a, T> {
        Self { slice, parent }
    }
    pub fn slice(&mut self) -> &mut [T] {
        self.slice
    }
    pub fn produce(self, n: usize, tags: &[Tag]) {
        self.parent.produce(n, tags);
    }
    pub fn len(&self) -> usize {
        self.slice.len()
    }
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
    state: Arc<Mutex<BufferState<T>>>,
    circ: Circ,
}

impl<T> Buffer<T> {
    /// Create a new Buffer.
    ///
    /// TODO: actually use the `size` parameter.
    pub fn new(size: usize) -> Result<Self> {
        assert_eq!(size, 4096);
        Ok(Self {
            state: Arc::new(Mutex::new(BufferState {
                read_borrow: false,
                write_borrow: false,
                rpos: 0,
                wpos: 0,
                used: 0,
                circ_len: size,
                member_size: std::mem::size_of::<T>(),
                noncopy: VecDeque::<T>::new(),
                tags: BTreeMap::new(),
            })),
            circ: Circ::new()?,
        })
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
        s.rpos = (s.rpos + n) % s.capacity();
        s.used -= n;
        // TODO: clean up tags.
    }

    /// Produce samples (commit writes).
    ///
    /// Will only be called from the write buffer.
    pub(in crate::circular_buffer) fn produce(&self, n: usize, tags: &[Tag]) {
        let mut s = self.state.lock().unwrap();
        assert!(s.free() >= n);
        assert!(
            s.write_capacity() >= n,
            "can't produce that much. {} < {}",
            s.write_capacity(),
            n
        );
        for tag in tags {
            let pos = (tag.pos() + s.wpos as u64) % s.capacity() as u64;
            let tag = Tag::new(pos, tag.key().into(), tag.val().clone());
            s.tags.entry(pos).or_insert_with(Vec::new).push(tag);
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
        let buf = self.circ.full_buffer::<T>();
        let (start, end) = s.read_range();
        let mut tags = Vec::new();

        // TODO: range scan the tags.
        for (n, ts) in &s.tags {
            let modded_n: usize = (*n % s.capacity() as TagPos).try_into().unwrap();
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
                    tag.pos() - start as TagPos,
                    tag.key().into(),
                    tag.val().clone(),
                ));
            }
        }
        Ok((
            BufferReader::new(unsafe { std::mem::transmute(&buf[start..end]) }, &self),
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
        let buf = self.circ.full_buffer::<T>();
        let (start, end) = s.write_range();
        Ok(BufferWriter::new(
            unsafe { std::mem::transmute(&mut buf[start..end]) },
            &self,
        ))
    }
}

// TODO: Can we have these only exist when *not* Copy?
impl<T> Buffer<T> {
    /// Push a value.
    pub fn push(&self, v: T) {
        let mut s = self.state.lock().unwrap();
        s.noncopy.push_back(v);
    }
    /// Push a value.
    pub fn pop(&self) -> Option<T> {
        let mut s = self.state.lock().unwrap();
        s.noncopy.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Float;
    use std::sync::{Arc, Mutex};

    #[test]
    pub fn test_no_double() -> Result<()> {
        let b = Arc::new(Buffer::<u8>::new(4096)?);
        {
            let _i1 = b.read_buf()?;
            assert!(matches![b.read_buf(), Err(_)]);
        }
        let _i2 = b.read_buf()?;
        {
            let _w1 = b.write_buf()?;
            assert!(matches![b.write_buf(), Err(_)]);
        }
        let _w2 = b.write_buf()?;
        Ok(())
    }

    #[test]
    pub fn test_typical() -> Result<()> {
        let b: Buffer<u8> = Buffer::new(4096)?;

        // Initial.
        assert!(b.read_buf()?.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);

        // Write a byte.
        {
            let mut buf = b.write_buf()?;
            buf.slice()[0] = 123;
            buf.produce(1, &vec![]);
            assert_eq!(b.read_buf()?.slice(), vec![123]);
            assert_eq!(b.write_buf()?.len(), 4095);
        }

        // Consume the byte.
        b.consume(1);
        assert!(b.read_buf()?.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);

        // Write towards the end bytes.
        {
            let n = 4000;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = (i & 0xff) as u8;
            }
            wb.produce(n, &vec![]);
            let rb = b.read_buf()?;
            assert_eq!(rb.len(), n);
            for i in 0..n {
                assert_eq!(rb.slice()[i], (i & 0xff) as u8);
            }
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
            wb.produce(n, &vec![]);
            let rb = b.read_buf()?;
            assert_eq!(rb.len(), n);
            for i in 0..n {
                assert_eq!(rb.slice()[i], ((n - i) & 0xff) as u8);
            }
        }
        assert_eq!(b.read_buf()?.len(), 100);
        assert_eq!(b.write_buf()?.len(), 3996);
        Ok(())
    }

    #[test]
    pub fn exact_overflow() -> Result<()> {
        let mut b: Buffer<u8> = Buffer::new(4096)?;

        // Initial.
        assert!(b.read_buf()?.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);

        // Full.
        b.write_buf()?.produce(4096, &vec![]);
        assert_eq!(b.read_buf()?.len(), 4096);
        assert_eq!(b.write_buf()?.len(), 0);

        // Empty again.
        b.read_buf()?.consume(4096);
        assert!(b.read_buf()?.is_empty());
        assert_eq!(b.write_buf()?.len(), 4096);
        Ok(())
    }

    #[test]
    pub fn test_float() -> Result<()> {
        let b: Buffer<Float> = Buffer::new(4096)?;

        // Initial.
        assert!(b.read_buf()?.is_empty());
        assert_eq!(b.write_buf()?.len(), 1024);

        // Write a sample.
        {
            let mut wb = b.write_buf()?;
            wb.slice()[0] = 123.321;
            wb.produce(1, &vec![]);
        }
        assert_eq!(b.read_buf()?.slice(), vec![123.321]);
        assert_eq!(b.write_buf()?.len(), 1023);

        // Consume the sample.
        b.read_buf()?.consume(1);
        assert!(b.read_buf()?.is_empty());
        assert_eq!(b.write_buf()?.len(), 1024);

        // Write towards the end bytes.
        {
            let n = 1000;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = i as Float;
            }
            wb.produce(n, &vec![]);
            assert_eq!(b.read_buf()?.len(), n);
            for i in 0..n {
                assert_eq!(b.read_buf()?.slice()[i], i as Float);
            }
            assert_eq!(b.write_buf()?.len(), 24);
        }
        b.read_buf()?.consume(1000);

        // Write 100 bytes.
        {
            let n = 100;
            let mut wb = b.write_buf()?;
            for i in 0..n {
                wb.slice()[i] = (n - i) as Float;
            }
            wb.produce(n, &vec![]);
            assert_eq!(b.read_buf()?.len(), n);
            for i in 0..n {
                assert_eq!(b.read_buf()?.slice()[i], (n - i) as Float);
            }
        }
        assert_eq!(b.read_buf()?.len(), 100);
        assert_eq!(b.write_buf()?.len(), 1024 - 100);
        Ok(())
    }
}
