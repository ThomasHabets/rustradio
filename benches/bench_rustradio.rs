#![feature(test)]

extern crate rustradio;
extern crate test;
use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::{new_streamp, Streamp};
use rustradio::Complex;

use test::Bencher;

#[bench]
fn bench_fft_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0);
    let input = {
        let mut a = taps.clone();
        a.resize(a.len() * 2, Complex::default());
        a
    };
    let s = new_streamp();
    let mut filter = FftFilter::new(s.clone(), &taps);
    b.iter(|| {
        s.lock().unwrap().clear();
        s.lock().unwrap().write_slice(&input);
        filter.out().lock().unwrap().clear();
        filter.work().unwrap();
    });
}

#[bench]
fn bench_fir_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0);
    let input = {
        let mut a = taps.clone();
        a.resize(a.len() * 2, Complex::default());
        a
    };
    let s = new_streamp();
    let mut filter = FIRFilter::new(s.clone(), &taps);
    b.iter(|| {
        s.lock().unwrap().clear();
        s.lock().unwrap().write_slice(&input);
        filter.out().lock().unwrap().clear();
        filter.work().unwrap();
    });
}
