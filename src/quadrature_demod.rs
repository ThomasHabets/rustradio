//! Quadrature demod, the core of an FM demodulator.
/*
TODO:
* Look into https://mazzo.li/posts/vectorized-atan2.html
*/
use anyhow::Result;
use log::info;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::volk::volk_32fc_s32f_atan2_32f;
use crate::{map_block_convert_macro, Complex, Error, Float};

/// Quadrature demod, the core of an FM demodulator.
pub struct QuadratureDemod {
    gain: Float,
    last: Complex,
}

impl QuadratureDemod {
    /// Create new QuadratureDemod block.
    ///
    /// Gain is just used to scale the value, and can be set to 1.0 if
    /// you don't care about the scale.
    pub fn new(gain: Float) -> Self {
        Self {
            gain,
            last: Complex::default(),
        }
    }
    fn process_one(&mut self, s: Complex) -> Float {
        let t = s * self.last.conj();
        self.last = s;
        self.gain * t.im.atan2(t.re)
    }
}

impl Block for QuadratureDemod {
    fn block_name(&self) -> &'static str {
        "QuadratureDemod"
    }
    fn work(&mut self, r: &mut InputStreams, w: &mut OutputStreams) -> Result<BlockRet, Error> {
        let i = r.get::<Complex>(0);
        if false {
            w.get(0)
                .borrow_mut()
                .write(i.borrow().iter().map(|x| self.process_one(*x)));
        } else {
            let mut idata = Vec::with_capacity(i.borrow().available() * 2);
            //info!("Quad len: {}", idata.capacity());
            for v in i.borrow().iter() {
                let i = *v * self.last.conj();
                idata.push(i.re);
                idata.push(i.im);
                self.last = *v;
            }
            let v: Vec<Float> = crate::volk::volk_32fc_s32f_atan2_32f_b(&idata, self.gain);
            //let v: Vec<Float>= idata.iter().map(|x| self.process_one(*x)).collect();
            w.get(0).borrow_mut().write_slice(&v);
        }
        i.borrow_mut().clear();
        Ok(BlockRet::Ok)
    }
}

//map_block_convert_macro![QuadratureDemod];
