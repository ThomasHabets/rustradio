//! Experimentation area for circular buffers.
use anyhow::Result;
use std::sync::{Arc, Mutex};

struct Buffer<T> {
    rpos: usize,
    wpos: usize,
    buf: Vec<T>,
}

impl<T: Default + std::fmt::Debug + Copy> Buffer<T> {
    fn new(size: usize) -> Self {
        Self {
            rpos: 0,
            wpos: 0,
            buf: vec![T::default(); size],
        }
    }

    fn consume(&mut self, n: usize) {
        assert!(
            self.rpos + n <= self.wpos,
            "Consumed too much: {} + {} <= {}",
            self.rpos,
            n,
            self.wpos
        );
        self.rpos += n;
    }
    fn produce(&mut self, n: usize) {
        assert!(self.buf.len() - self.wpos >= n, "can't produce that much");
        self.wpos += n;
    }

    fn read_buf(&mut self) -> &'static [T] {
        unsafe { std::mem::transmute(&self.buf[self.rpos..self.wpos]) }
    }
    fn write_buf(&mut self) -> &'static mut [T] {
        unsafe { std::mem::transmute(&mut self.buf[self.wpos..]) }
    }
}

fn main() -> Result<()> {
    let b = Arc::new(Mutex::new(Buffer::new(10)));

    let b2 = b.clone();
    std::thread::spawn(move || loop {
        let rb = b2.lock().unwrap().read_buf();
        println!("read buf: {:?}", rb);
        b2.lock().unwrap().consume(rb.len());
        std::thread::sleep(std::time::Duration::from_millis(200));
    });

    let mut n = 0;
    loop {
        let wb = b.lock().unwrap().write_buf();
        if !wb.is_empty() {
            wb[0] = n;
            n += 1;
            println!("w capacity: {:?}", wb.len());
            b.lock().unwrap().produce(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
