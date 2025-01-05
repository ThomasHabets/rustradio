#![feature(test)]

extern crate rustradio;
extern crate test;
use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::new_stream;
use rustradio::window::WindowType;
use rustradio::Complex;

use test::Bencher;

#[bench]
fn bench_fft_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0, &WindowType::Hamming);
    let input = {
        let mut a = taps.clone();
        a.resize(a.len() * 2, Complex::default());
        a
    };
    let (sw, sr) = new_stream();
    let (mut filter, out) = FftFilter::new(sr, &taps);
    b.iter(|| {
        // Fill input buffer.
        {
            let o = sw.write_buf().unwrap();
            //o.slice()[..input.len()].clone_from_slice(&input);
            o.produce(input.len(), &[]);
        }
        // Empty output buffer.
        {
            let (out, _) = out.read_buf().unwrap();
            let n = out.len();
            out.consume(n);
        }
        filter.work().unwrap();
    });
}

#[bench]
fn bench_fir_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0, &WindowType::Hamming);
    let input = {
        let mut a = taps.clone();
        a.resize(a.len() * 2, Complex::default());
        a
    };
    let (sw, sr) = new_stream();
    let (mut filter, out) = FIRFilter::new(sr, &taps);
    b.iter(|| {
        // Fill input buffer.
        {
            let o = sw.write_buf().unwrap();
            //o.slice()[..input.len()].clone_from_slice(&input);
            o.produce(input.len(), &[]);
        }
        // Empty output buffer.
        {
            let (out, _) = out.read_buf().unwrap();
            let n = out.len();
            out.consume(n);
        }
        filter.work().unwrap();
    });
}
