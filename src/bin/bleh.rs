use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;

use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::{Float, InputStreams, OutputStreams, Stream, StreamType, Streamp};

fn main() -> Result<()> {
    println!("Hello, world!");

    {
        let s = StreamType::Float(Rc::new(RefCell::new(Stream::<Float>::new_from_slice(&[
            1.0, -1.0, 3.9,
        ]))));
        let mut is = InputStreams::new();
        is.add_stream(s);
        let mut add = AddConst::new(1.1);

        let s = StreamType::Float(Rc::new(RefCell::new(Stream::<Float>::new())));
        let mut os = OutputStreams::new();
        os.add_stream(s);

        add.work(&mut is, &mut os)?;
        let res: Streamp<Float> = os.get(0).into();
        println!("{:?}", &res.borrow().iter().collect::<Vec<&Float>>());
    }

    Ok(())
}
