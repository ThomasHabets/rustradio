/*! volk FFI wrapper.
 */
/*
This file should probably be generated.
*/
use libc::{c_float, c_uint};
#[allow(non_camel_case_types)]

type t_volk_32fc_s32f_atan2_32f =
    unsafe extern "C" fn(*mut c_float, *const c_float, c_float, c_uint);

type t_volk_32fc_x2_multiply_conjugate_32fc =
    unsafe extern "C" fn(*mut c_float, *const c_float, *const c_float, c_uint);

#[link(name = "volk")]
extern "C" {
    pub static volk_32fc_s32f_atan2_32f: t_volk_32fc_s32f_atan2_32f;
    pub static volk_32fc_x2_multiply_conjugate_32fc: t_volk_32fc_x2_multiply_conjugate_32fc;
}
