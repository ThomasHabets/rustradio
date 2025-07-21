//! Experimentation area for circular buffers.
use anyhow::Result;

use rustradio::sys::circular_buffer::Buffer;
use std::sync::Arc;

fn main() -> Result<()> {
    let b = Arc::new(Buffer::new(4096)?);

    let b2 = b.clone();
    std::thread::spawn(move || {
        loop {
            let (rb, _) = Arc::clone(&b2).read_buf().unwrap();
            println!("read buf: {:?}", rb.slice());
            let l = rb.slice().len();
            rb.consume(l);
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    });

    let mut n = 0;
    loop {
        let mut wb = Arc::clone(&b).write_buf().unwrap();
        if !wb.slice().is_empty() {
            wb.slice()[0] = n;
            n += 1;
            println!("w capacity: {:?}", wb.slice().len());
            let l = wb.slice().len();
            wb.produce(l, &[]);
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
