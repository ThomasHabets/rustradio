#![feature(test)]

extern crate rustradio;
extern crate test;
use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::{InputStreams, OutputStreams, StreamType};
use rustradio::Complex;

use test::Bencher;

#[bench]
fn bench_fft_filter(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass(1024000.0, 50000.0, 10000.0);
    let input = {
        let mut a = taps.clone();
        a.resize(a.len() * 2, Complex::default());
        a
    };
    let mut filter = FftFilter::new(&taps);
    let stream_in = StreamType::new_complex();
    let stream_out = StreamType::new_complex();
    let mut is = InputStreams::new();
    let mut os = OutputStreams::new();
    is.add_stream(stream_in.clone());
    os.add_stream(stream_out.clone());
    b.iter(|| {
        if let StreamType::Complex(x) = &stream_in {
            x.borrow_mut().clear();
            x.borrow_mut().write_slice(&input);
        }
        if let StreamType::Complex(x) = &stream_out {
            x.borrow_mut().clear();
        }
        while is.available(0) > 0 {
            filter.work(&mut is, &mut os).unwrap();
        }
    });
}

#[bench]
fn bench_fir(b: &mut Bencher) {
    let taps = rustradio::fir::low_pass(1024000.0, 50000.0, 10000.0);
    let input = {
        let mut a = taps.clone();
        a.resize(a.len() * 2, Complex::default());
        a
    };
    let mut filter = FIRFilter::new(&taps);
    let stream_in = StreamType::new_complex();
    let stream_out = StreamType::new_complex();
    let mut is = InputStreams::new();
    let mut os = OutputStreams::new();
    is.add_stream(stream_in.clone());
    os.add_stream(stream_out.clone());
    b.iter(|| {
        if let StreamType::Complex(x) = &stream_in {
            x.borrow_mut().clear();
            x.borrow_mut().write_slice(&input);
        }
        if let StreamType::Complex(x) = &stream_out {
            x.borrow_mut().clear();
        }
        let mut last = is.available(0);
        loop {
            filter.work(&mut is, &mut os).unwrap();
            let nxt = is.available(0);
            if last == nxt {
                break;
            }
            last = nxt;
        }
    });
}
