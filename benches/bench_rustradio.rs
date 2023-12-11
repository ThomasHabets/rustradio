extern crate rustradio;

use criterion::{criterion_group, criterion_main, Criterion};

use rustradio::block::Block;
use rustradio::blocks::*;
use rustradio::stream::{new_streamp, Streamp};

fn setup_empty<T: Copy, T2: Copy>(s: &Streamp<T>, so: &Streamp<T2>) {
    let len = s.len();
    // Empty input buffer.
    {
        let (i, _) = s.read_buf().expect("getting read buffer");
        let n = i.len();
        i.consume(n);
    }
    // Fill input buffer.
    {
        let o = s.write_buf().expect("getting read buffer");
        o.produce(len, &[]);
    }
    // Empty output buffer.
    {
        let obind = so;
        let (out, _) = obind.read_buf().expect("getting write buffer");
        let n = out.len();
        out.consume(n);
    }
}

fn run_simple<Tin: Copy, Tout: Copy>(
    c: &mut Criterion,
    s: Streamp<Tin>,
    so: Streamp<Tout>,
    block: &mut dyn Block,
) {
    c.bench_function(block.block_name(), |b| {
        b.iter(|| {
            setup_empty(&s, &so);
            let st = std::time::Instant::now();
            block.work().unwrap();
            st.elapsed()
        })
    });
}

fn bench_fast_fm(c: &mut Criterion) {
    let s = new_streamp();
    let mut block = FastFM::new(s.clone());
    run_simple(c, s, block.out(), &mut block);
}

fn bench_quad_demod(c: &mut Criterion) {
    let s = new_streamp();
    let mut block = QuadratureDemod::new(s.clone(), 1.0);
    run_simple(c, s, block.out(), &mut block);
}

fn bench_fft_filter(c: &mut Criterion) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0);
    let s = new_streamp();
    let mut block = FftFilter::new(s.clone(), &taps);
    run_simple(c, s, block.out(), &mut block);
}

fn bench_fir_filter(c: &mut Criterion) {
    let taps = rustradio::fir::low_pass_complex(1024000.0, 50000.0, 10000.0);
    let s = new_streamp();
    let mut block = FIRFilter::new(s.clone(), &taps);
    run_simple(c, s, block.out(), &mut block);
}

criterion_group!(
    benches,
    bench_quad_demod,
    bench_fast_fm,
    bench_fft_filter,
    bench_fir_filter,
);
criterion_main!(benches);
