pub mod circular_buffer;

/// System page size in bytes.
pub(crate) fn page_size() -> crate::Result<usize> {
    // SAFETY: sysconf takes no pointers and reads the system page size.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(page_size)
        .ok()
        .filter(|&p| p > 0)
        .ok_or_else(|| crate::Error::msg("unable to read the system page size"))
}

#[must_use]
pub(crate) fn get_cpu_time() -> std::time::Duration {
    use libc::{CLOCK_PROCESS_CPUTIME_ID, clock_gettime, timespec};
    // SAFETY: Zeroing out a timespec struct is just all zeroes.
    let mut ts: timespec = unsafe { std::mem::zeroed() };
    // SAFETY: Local variable written my C function.
    let rc = unsafe { clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    if rc != 0 {
        panic!("clock_gettime()");
    }
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

pub(crate) fn sleep(d: std::time::Duration) {
    if !cfg!(feature = "wasm") {
        std::thread::sleep(d);
    }
}

pub mod export {
    pub use super::circular_buffer;
    pub(crate) use super::get_cpu_time;
    pub(crate) use super::page_size;
    pub(crate) use super::sleep;
    pub use std::time::Instant;
    pub type Buffer<T> = circular_buffer::Buffer<T>;
    pub type BufferReader<T> = circular_buffer::BufferReader<T>;
    pub type BufferWriter<T> = circular_buffer::BufferWriter<T>;
}
