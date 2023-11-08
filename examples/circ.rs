//! Experimentation area for circular buffers.
use anyhow::Result;

use rustradio::circular_buffer::Buffer;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    let b = Arc::new(Buffer::new(4096)?);

    let b2 = b.clone();
    std::thread::spawn(move || loop {
        let rb = b2.read_buf();
        println!("read buf: {:?}", rb);
        b2.consume(rb.len());
        std::thread::sleep(std::time::Duration::from_millis(1000));
    });

    let mut n = 0;
    loop {
        let wb = b.write_buf();
        if !wb.is_empty() {
            wb[0] = n;
            n += 1;
            println!("w capacity: {:?}", wb.len());
            b.produce(wb.len());
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
