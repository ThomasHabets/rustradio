use anyhow::Result;

use crate::block::{get_input, get_output, Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams};
use crate::{Complex, Error, Float};

pub struct RtlSdrDecode;

impl RtlSdrDecode {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for RtlSdrDecode {
    fn default() -> Self {
        Self::new()
    }
}

impl Block for RtlSdrDecode {
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let samples: usize = r.available(0) - r.available(0) % 2;
        let input = get_input(r, 0);
        let out = get_output(w, 0);

        // TODO: needless copy.
        let buf: Vec<u8> = input.borrow().data().clone().into();
        let buf = &buf[..samples];
        out.borrow_mut().write_slice(
            (0..samples)
                .step_by(2)
                .map(|e| {
                    Complex::new(
                        ((buf[e] as Float) - 127.0) * 0.008,
                        ((buf[e + 1] as Float) - 127.0) * 0.008,
                    )
                })
                .collect::<Vec<Complex>>()
                .as_slice(),
        );
        input.borrow_mut().consume(samples);
        Ok(BlockRet::Ok)
    }
}
