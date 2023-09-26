use anyhow::Result;

use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use rustradio::Float;

fn main() -> Result<()> {
    println!("Hello, world!");

    {
        let s = StreamType::new_float_from_slice(&[1.0, -1.0, 3.9]);
        let mut is = InputStreams::new();
        is.add_stream(s);
        let mut add = AddConst::new(1.1);

        let s = StreamType::new_float();
        let mut os = OutputStreams::new();
        os.add_stream(s);

        add.work(&mut is, &mut os)?;
        let res: Streamp<Float> = os.get(0).into();
        println!("{:?}", &res.borrow().iter().collect::<Vec<&Float>>());
    }

    Ok(())
}
