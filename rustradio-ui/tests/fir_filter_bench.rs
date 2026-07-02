use rustradio::blocks::{FirFilter, NullSink, VectorSource};
use rustradio::fir::low_pass_complex;
use rustradio::graph::{Graph, GraphRunner};
use rustradio::window::WindowType;
use rustradio::{Complex, Float, Repeat};
use std::time::Duration;
use wasm_bindgen_test::{
    Criterion, wasm_bindgen_bench, wasm_bindgen_test, wasm_bindgen_test_configure,
};

wasm_bindgen_test_configure!(run_in_browser);

const INPUT_SAMPLES: usize = 1024 * 1024;
const REPEATS: u64 = 2;

fn performance_now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_default()
}

#[wasm_bindgen_test]
fn fir_filter_graph_smoke() {
    run_fir_filter_graph();
}

#[wasm_bindgen_bench]
fn fir_filter_graph_benchmark(c: &mut Criterion) {
    *c = std::mem::take(c)
        .sample_size(10)
        .warm_up_time(Duration::from_millis(50))
        .measurement_time(Duration::from_millis(250));
    c.bench_function("fir_filter_graph", |b| {
        b.iter(run_fir_filter_graph);
    });
}

fn run_fir_filter_graph() {
    let input = (0..INPUT_SAMPLES)
        .map(|n| {
            let phase = n as Float * 0.013;
            Complex::new(phase.sin(), phase.cos())
        })
        .collect::<Vec<_>>();
    let taps = low_pass_complex(48_000.0, 4_000.0, 1_000.0, WindowType::Hamming);
    let taps_len = taps.len();

    let (src, src_out) = VectorSource::builder(input)
        .repeat(Repeat::finite(REPEATS))
        .build()
        .expect("build vector source");
    let (fir, fir_out) = FirFilter::new(src_out, taps);
    let sink = NullSink::new(fir_out);

    let mut graph = Graph::new();
    graph.add(Box::new(src));
    graph.add(Box::new(fir));
    graph.add(Box::new(sink));

    let start = performance_now();
    graph.run().expect("run FIR benchmark graph");
    let elapsed_ms = performance_now() - start;
    let samples = INPUT_SAMPLES as f64 * REPEATS as f64;
    let ms_per_msps = elapsed_ms / (samples / 1_000_000.0);

    web_sys::console::log_1(
        &format!(
            "FirFilter graph benchmark: {samples:.0} samples, {taps_len} taps, {elapsed_ms:.3} ms, {ms_per_msps:.3} ms/Msample"
        )
        .into(),
    );
    assert!(elapsed_ms >= 0.0);
}
