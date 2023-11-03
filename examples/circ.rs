//! Experimentation area for circular buffers.
use anyhow::Result;

use rustradio::circular_buffer::Buffer;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    let b = Arc::new(Mutex::new(Buffer::new(10)?));

    let b2 = b.clone();
    std::thread::spawn(move || loop {
        let rb = b2.lock().unwrap().read_buf();
        println!("read buf: {:?}", rb);
        b2.lock().unwrap().consume(rb.len());
        std::thread::sleep(std::time::Duration::from_millis(1000));
    });

    let mut n = 0;
    loop {
        let wb = b.lock().unwrap().write_buf();
        if !wb.is_empty() {
            wb[0] = n;
            n += 1;
            println!("w capacity: {:?}", wb.len());
            b.lock().unwrap().produce(wb.len());
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
