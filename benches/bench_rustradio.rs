#![feature(test)]

extern crate rustradio;
extern crate test;
use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::stream::new_stream;
use rustradio::window::WindowType;
use rustradio::Complex;

use test::Bencher;

/// AVX2 code for multiplying two vectors.
///
/// Looks like this code is a little slower on my laptop than the rust compiled code, as of Rust
/// 1.83.0.
///
/// ```
/// test bench_sum_vec      ... bench:      37,735.72 ns/iter (+/- 2,168.16)
/// test bench_sum_vec_avx2 ... bench:      42,250.73 ns/iter (+/- 23,112.96)
/// ```
#[cfg(target_feature = "avx2")]
fn sum_vec_avx2(left: &[Complex], right: &[Complex]) -> Vec<Complex> {
    use std::mem::MaybeUninit;
    let len = left.len();
    let mut ret: Vec<MaybeUninit<Complex>> = Vec::with_capacity(len);
    let ret = unsafe {
        ret.set_len(left.len());
        std::mem::transmute::<Vec<MaybeUninit<Complex>>, Vec<Complex>>(ret)
    };
    (0..len).step_by(4).for_each(|i| unsafe {
        use core::arch::x86_64::*;
        let a = _mm256_loadu_ps((left.as_ptr() as *const f32).add(i * 2));
        let b = _mm256_loadu_ps((right.as_ptr() as *const f32).add(i * 2));
        let a_re = _mm256_shuffle_ps(a, a, 0b10001000);
        let a_im = _mm256_shuffle_ps(a, a, 0b11011101);
        let b_re = _mm256_shuffle_ps(b, b, 0b10001000);
        let b_im = _mm256_shuffle_ps(b, b, 0b11011101);
        let re = _mm256_fmsub_ps(a_re, b_re, _mm256_mul_ps(a_im, b_im));
        let im = _mm256_fmadd_ps(a_re, b_im, _mm256_mul_ps(a_im, b_re));
        let res = _mm256_unpacklo_ps(re, im);
        _mm256_storeu_ps((ret.as_ptr() as *mut f32).add(i * 2), res);
    });
    ret
}

fn sum_vec(left: &[Complex], right: &[Complex]) -> Vec<Complex> {
    left.iter().zip(right.iter()).map(|(x, y)| x * y).collect()
}

#[bench]
fn bench_sum_vec(b: &mut Bencher) {
    let n = 102400;
    let left = vec![Complex::default(); n];
    b.iter(|| sum_vec(&left, &left));
}

#[cfg(target_feature = "avx2")]
#[bench]
fn bench_sum_vec_avx2(b: &mut Bencher) {
    let n = 102400;
    let left = vec![Complex::default(); n];
    b.iter(|| sum_vec_avx2(&left, &left));
}

#[bench]
fn bench_fft_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0, &WindowType::Hamming);
    let (sw, sr) = new_stream();
    let (mut filter, out) = FftFilter::new(sr, &taps);
    b.iter(|| {
        // Fill input buffer.
        {
            let free = sw.free();
            let o = sw.write_buf().unwrap();
            //o.slice()[..input.len()].clone_from_slice(&input);
            o.produce(free, &[]);
        }
        // Empty output buffer.
        {
            let (out, _) = out.read_buf().unwrap();
            let n = out.len();
            out.consume(n);
        }
        assert_eq!(BlockRet::Ok, filter.work().unwrap());
        assert_eq!(BlockRet::Noop, filter.work().unwrap());
    });
}

#[bench]
fn bench_fir_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0, &WindowType::Hamming);
    let (sw, sr) = new_stream();
    let (mut filter, out) = FIRFilter::new(sr, &taps);
    b.iter(|| {
        // Fill input buffer.
        {
            let free = sw.free();
            let o = sw.write_buf().unwrap();
            //o.slice()[..input.len()].clone_from_slice(&input);
            o.produce(free, &[]);
        }
        // Empty output buffer.
        {
            let (out, _) = out.read_buf().unwrap();
            let n = out.len();
            out.consume(n);
        }
        loop {
            match filter.work().unwrap() {
                BlockRet::Ok => continue,
                BlockRet::Noop => break,
                other => panic!("FIRFilter returned {other:?}"),
            }
        }
    });
}
