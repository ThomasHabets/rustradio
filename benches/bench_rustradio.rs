#![feature(test)]

extern crate rustradio;
extern crate test;
use rustradio::Complex;
use rustradio::block::{Block, BlockRet};
use rustradio::blocks::*;
use rustradio::stream::new_stream;
use rustradio::window::WindowType;

use test::Bencher;

/// AVX code for multiplying two vectors.
///
/// Looks like this code is a little faster on my laptop than the rust compiled code, as of Rust
/// nightly (cargo 1.86.0-nightly (2928e3273 2025-02-07)).
///
/// ```
/// test bench_sum_vec         ... bench:      49,523.45 ns/iter (+/- 2,023.08)
/// test bench_sum_vec_avx_fma ... bench:      43,474.55 ns/iter (+/- 1,735.46)
/// ```
#[cfg(all(target_feature = "avx", target_feature = "fma"))]
fn sum_vec_avx_fma(left: &[Complex], right: &[Complex]) -> Vec<Complex> {
    use std::mem::MaybeUninit;
    let len = left.len();
    let mut ret: Vec<MaybeUninit<Complex>> = Vec::with_capacity(len);
    // SAFETY:
    // We're filling all of this.
    let ret = unsafe {
        ret.set_len(left.len());
        std::mem::transmute::<Vec<MaybeUninit<Complex>>, Vec<Complex>>(ret)
    };
    // SAFETY:
    // SIMD stuff, always unsafe in theory.
    (0..len).step_by(4).for_each(|i| unsafe {
        // All instrucions are AVX except fmsub/fmadd.
        use core::arch::x86_64::*;
        let a = _mm256_loadu_ps((left.as_ptr() as *const f32).add(i * 2));
        let b = _mm256_loadu_ps((right.as_ptr() as *const f32).add(i * 2));
        let a_re = _mm256_shuffle_ps(a, a, 0b10001000);
        let a_im = _mm256_shuffle_ps(a, a, 0b11011101);
        let b_re = _mm256_shuffle_ps(b, b, 0b10001000);
        let b_im = _mm256_shuffle_ps(b, b, 0b11011101);
        // fmsub_ps and add is tagged `fma`.
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

#[cfg(all(target_feature = "avx", target_feature = "fma"))]
#[bench]
fn bench_sum_vec_avx_fma(b: &mut Bencher) {
    let n = 102400;
    let left = vec![Complex::default(); n];
    b.iter(|| sum_vec_avx_fma(&left, &left));
}

#[bench]
fn bench_fft_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0, &WindowType::Hamming);
    let (sw, sr) = new_stream();
    let (mut filter, out) = FftFilter::new(sr, taps);
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
        assert!(matches![
            filter.work().unwrap(),
            BlockRet::WaitForStream(_, _)
        ]);
    });
}

#[bench]
fn bench_fir_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0, &WindowType::Hamming);
    let (sw, sr) = new_stream();
    let (mut filter, out) = FirFilter::new(sr, &taps);
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
                BlockRet::Again => continue,
                BlockRet::WaitForStream(_, _) => break,
                _other => panic!("FirFilter returned bad state"),
            }
        }
    });
}

fn bench_sync(b: &mut Bencher, tagged: bool, tag_aware: bool) {
    use rustradio::stream::{Tag, TagValue};
    use std::borrow::Cow;

    let samples = vec![1u32; 8192];
    let tags: Vec<_> = if tagged {
        (0..samples.len())
            .step_by(256)
            .map(|pos| Tag::new(pos, "marker", TagValue::U64(pos as u64)))
            .collect()
    } else {
        vec![]
    };
    let (writer, reader) = new_stream();
    let (mut block, output): (Box<dyn Block>, _) = if tag_aware {
        let (block, output) = Map::new(reader, "increment", |sample, tags| {
            (sample + 1, Cow::Borrowed(tags))
        });
        (Box::new(block), output)
    } else {
        let (block, output) = AddConst::new(reader, 1u32);
        (Box::new(block), output)
    };
    b.bytes = (samples.len() * std::mem::size_of::<u32>()) as u64;
    b.iter(|| {
        let mut window = writer.write_buf().unwrap();
        window.fill_from_slice(&samples);
        window.produce(samples.len(), &tags);
        assert!(matches!(
            block.work().unwrap(),
            BlockRet::WaitForStream(_, 1)
        ));
        let (window, tags) = output.read_buf().unwrap();
        std::hint::black_box(window.slice());
        std::hint::black_box(tags);
        window.consume(samples.len());
    });
}

#[bench]
fn bench_sync_plain_untagged(b: &mut Bencher) {
    bench_sync(b, false, false);
}

#[bench]
fn bench_sync_plain_tagged(b: &mut Bencher) {
    bench_sync(b, true, false);
}

#[bench]
fn bench_sync_tag_aware_untagged(b: &mut Bencher) {
    bench_sync(b, false, true);
}

#[bench]
fn bench_sync_tag_aware_tagged(b: &mut Bencher) {
    bench_sync(b, true, true);
}

fn bench_serialize(b: &mut Bencher, append: bool) {
    use rustradio::Sample;

    let samples = vec![Complex::new(1.25, -2.5); 8192];
    let mut bytes = Vec::with_capacity(samples.len() * Complex::size());
    b.bytes = bytes.capacity() as u64;
    b.iter(|| {
        bytes.clear();
        for sample in std::hint::black_box(&samples) {
            if append {
                sample.serialize_into(&mut bytes);
            } else {
                bytes.extend_from_slice(&sample.serialize());
            }
        }
        std::hint::black_box(&bytes);
    });
}

#[bench]
fn bench_serialize_samples(b: &mut Bencher) {
    bench_serialize(b, false);
}

#[bench]
fn bench_serialize_samples_into(b: &mut Bencher) {
    bench_serialize(b, true);
}
