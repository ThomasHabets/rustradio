/*! volk FFI wrapper.
 */
/*
This file should probably be generated.
*/
use crate::{Complex, Float};
use libc::{c_float, c_uint};

mod volk_ffi;

#[allow(non_camel_case_types)]

/// volk_32fc_s32f_atan2_32f()
pub fn volk_32fc_s32f_atan2_32f(input: &[Complex], scale: Float) -> Vec<Float> {
    let n = input.len();
    let mut o: Vec<c_float> = Vec::with_capacity(n);
    let mut i: Vec<c_float> = Vec::with_capacity(n * 2);

    input.iter().for_each(|c| {
        i.push(c.re);
        i.push(c.im);
    });
    assert_eq!(i.len(), o.len() * 2);
    unsafe {
        volk_ffi::volk_32fc_s32f_atan2_32f(
            o.as_mut_ptr(),
            i.as_ptr(),
            scale as c_float,
            n as c_uint,
        );
        o.set_len(n);
    };
    o
}

pub fn volk_32fc_s32f_atan2_32f_b(input: &[c_float], scale: Float) -> Vec<Float> {
    let n = input.len() / 2;
    let mut o: Vec<c_float> = Vec::with_capacity(n);
    unsafe {
        volk_ffi::volk_32fc_s32f_atan2_32f(
            o.as_mut_ptr(),
            input.as_ptr(),
            scale as c_float,
            n as c_uint,
        );
        o.set_len(n);
    };
    o
}

pub fn volk_32fc_x2_multiply_conjugate_32fc(input: &[Float]) -> Vec<Float> {
    let n = input.len();
    let mut o: Vec<c_float> = Vec::with_capacity(n);
    unsafe {
        volk_ffi::volk_32fc_x2_multiply_conjugate_32fc(
            o.as_mut_ptr(),
            input.as_ptr(),
            input[2..].as_ptr(),
            ((n / 2) - 1) as c_uint,
        );
        o.set_len(n);
    };
    o
}
