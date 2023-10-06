//! Quadrature demod, the core of an FM demodulator.
/*
TODO:
* Look into https://mazzo.li/posts/vectorized-atan2.html
*/
use anyhow::Result;
use log::info;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType, Streamp};
use crate::volk::{
    volk_32fc_s32f_atan2_32f,
    volk_32fc_s32f_atan2_32f_b, // Special version.
    volk_32fc_x2_multiply_conjugate_32fc,
};
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
        if i.borrow().available() == 0 {
            return Ok(BlockRet::Ok);
        }

        if false {
            // This is the control. No volk here.
            w.get(0)
                .borrow_mut()
                .write(i.borrow().iter().map(|x| self.process_one(*x)));
        } else {
            // Volk code. Two parts. conjugate multiply, and then atan2.

            // 1. Conjugate multiply.
            let mut idata: Vec<Float> = Vec::with_capacity(i.borrow().available());
            for v in i.borrow().iter() {
                idata.push(v.re);
                idata.push(v.im);
            }

            // 2. atan2
            let idata = volk_32fc_x2_multiply_conjugate_32fc(&idata);
            let v: Vec<Float> = if true {
                let nt = idata.len();
                let mut v = Vec::with_capacity(nt / 2);
                for n in (0..nt).step_by(2) {
                    let t = Complex::new(idata[n], idata[n + 1]);
                    v.push(self.gain * t.im.atan2(t.re));
                }
                v
            } else {
                volk_32fc_s32f_atan2_32f_b(&idata, self.gain)
            };
            w.get(0).borrow_mut().write_slice(&v);
        }
        i.borrow_mut().clear();
        Ok(BlockRet::Ok)
    }
}

//map_block_convert_macro![QuadratureDemod];
