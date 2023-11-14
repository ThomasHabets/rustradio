#![feature(test)]

extern crate rustradio;
extern crate test;
use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::new_streamp;
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
        // Empty input buffer.
        {
            let (i, _) = s.read_buf().unwrap();
            let n = i.len();
            i.consume(n);
        }
        // Fill input buffer.
        {
            let o = s.write_buf().unwrap();
            //o.slice()[..input.len()].clone_from_slice(&input);
            o.produce(input.len(), &[]);
        }
        // Empty output buffer.
        {
            let obind = filter.out();
            let (out, _) = obind.read_buf().unwrap();
            let n = out.len();
            out.consume(n);
        }
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
        // Empty input buffer.
        {
            let (i, _) = s.read_buf().unwrap();
            let n = i.len();
            i.consume(n);
        }
        // Fill input buffer.
        {
            let o = s.write_buf().unwrap();
            //o.slice()[..input.len()].clone_from_slice(&input);
            o.produce(input.len(), &[]);
        }
        // Empty output buffer.
        {
            let obind = filter.out();
            let (out, _) = obind.read_buf().unwrap();
            let n = out.len();
            out.consume(n);
        }
        filter.work().unwrap();
    });
}
