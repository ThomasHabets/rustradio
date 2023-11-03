//! Test implementation of circular buffers.
//! Full of unsafe.

use anyhow::Result;
use std::os::fd::AsRawFd;

use libc::{c_int, c_uchar, c_void, off_t, size_t};
use libc::{MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};

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
    fn full_buffer<T>(&self) -> &'static mut [T] {
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

/// Type aware buffer.
pub struct Buffer<T> {
    rpos: usize,
    wpos: usize,
    //buf: Vec<T>,
    circ: Circ,
    dummy: std::marker::PhantomData<T>,
}

impl<T: Default + std::fmt::Debug + Copy> Buffer<T> {
    /// Create a new Buffer.
    ///
    /// TODO: actually use the `size` parameter.
    pub fn new(_size: usize) -> Result<Self> {
        Ok(Self {
            rpos: 0,
            wpos: 0,
            //buf: vec![T::default(); size],
            circ: Circ::new()?,
            dummy: std::marker::PhantomData,
        })
    }

    /// Consume samples from input buffer.
    pub fn consume(&mut self, n: usize) {
        assert!(
            self.rpos + n <= self.wpos,
            "Consumed too much: {} + {} <= {}",
            self.rpos,
            n,
            self.wpos
        );
        self.rpos = (self.rpos + n) % self.circ.len();
    }

    /// Produce samples (commit writes).
    pub fn produce(&mut self, n: usize) {
        assert!(
            self.write_capacity() >= n,
            "can't produce that much. {} < {}",
            self.write_capacity(),
            n
        );
        self.wpos = (self.wpos + n) % self.circ.len();
    }

    fn write_capacity(&self) -> usize {
        let (a, b) = self.write_range();
        b - a
    }

    fn write_range(&self) -> (usize, usize) {
        //eprintln!("Write range: {} {}", self.rpos, self.wpos);
        if self.rpos <= self.wpos {
            (self.wpos, self.rpos + self.circ.len())
        } else {
            (self.wpos, self.rpos)
        }
    }

    fn read_range(&self) -> (usize, usize) {
        let start = self.rpos;
        let end = self.wpos;
        if start <= end {
            (start, end)
        } else {
            (start, end + 4096)
        }
    }

    /// Get the read slice.
    pub fn read_buf(&mut self) -> &'static [T] {
        let buf = self.circ.full_buffer::<T>();
        let (start, end) = self.read_range();
        unsafe { std::mem::transmute(&buf[start..end]) }
    }

    /// Get the write slice.
    pub fn write_buf(&mut self) -> &'static mut [T] {
        let buf = self.circ.full_buffer::<T>();
        let (start, end) = self.write_range();
        unsafe { std::mem::transmute(&mut buf[start..end]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    pub fn test_typical() -> Result<()> {
        let mut b: Buffer<u8> = Buffer::new(20)?;

        // Initial.
        assert!(b.read_buf().is_empty());
        assert_eq!(b.write_buf().len(), 4096);

        // Write a byte.
        b.write_buf()[0] = 123;
        b.produce(1);
        assert_eq!(b.read_buf(), vec![123]);
        assert_eq!(b.write_buf().len(), 4095);

        // Consume the byte.
        b.consume(1);
        assert!(b.read_buf().is_empty());
        assert_eq!(b.write_buf().len(), 4096);

        // Write towards the end bytes.
        {
            let n = 4000;
            for i in 0..n {
                b.write_buf()[i] = (i & 0xff) as u8;
            }
            b.produce(n);
            assert_eq!(b.read_buf().len(), 4000);
            for i in 0..n {
                assert_eq!(b.read_buf()[i], (i & 0xff) as u8);
            }
            assert_eq!(b.write_buf().len(), 96);
        }
        b.consume(4000);

        // Write 100 bytes.
        {
            let n = 100;
            for i in 0..n {
                b.write_buf()[i] = ((n - i) & 0xff) as u8;
            }
            b.produce(n);
            assert_eq!(b.read_buf().len(), n);
            for i in 0..n {
                assert_eq!(b.read_buf()[i], ((n - i) & 0xff) as u8);
            }
        }
        assert_eq!(b.read_buf().len(), 100);
        assert_eq!(b.write_buf().len(), 3996);
        Ok(())
    }
}
