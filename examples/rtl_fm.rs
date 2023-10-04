use anyhow::Result;

use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::stream::StreamType;
use rustradio::Float;

fn main() -> Result<()> {
    println!("Running rtl_fm example");
    let mut g = Graph::new();

    let freq = 100_000_000;
    let samp_rate = 1024_000.0;
    let igain = 20;

    // RTL SDR source.
    let src = g.add(Box::new(RtlSdrSource::new(freq, samp_rate as u32, igain)?));
    let dec = g.add(Box::new(RtlSdrDecode::new()));
    g.connect(StreamType::new_u8(), src, 0, dec, 0);

    // Filter.
    let taps = rustradio::fir::low_pass(samp_rate, 100_000.0, 1000.0);
    let filter = g.add(Box::new(FftFilter::new(&taps)));
    g.connect(StreamType::new_complex(), dec, 0, filter, 0);

    // Resample.
    let new_samp_rate = 200_000.0;
    let resamp = g.add(Box::new(RationalResampler::new(
        new_samp_rate as usize,
        samp_rate as usize,
    )?));
    g.connect(StreamType::new_complex(), filter, 0, resamp, 0);
    let samp_rate = new_samp_rate;

    // Quad demod.
    let quad = g.add(Box::new(QuadratureDemod::new(1.0)));
    g.connect(StreamType::new_complex(), resamp, 0, quad, 0);

    // Resample audio.
    let new_samp_rate = 48_000.0;
    let audio_resamp = g.add(Box::new(RationalResampler::new(
        new_samp_rate as usize,
        samp_rate as usize,
    )?));
    let _samp_rate = new_samp_rate;
    g.connect(StreamType::new_float(), quad, 0, audio_resamp, 0);

    // Save to file.
    let sink = g.add(Box::new(FileSink::<Float>::new(
        "test.f32",
        Mode::Overwrite,
    )?));
    g.connect(StreamType::new_float(), audio_resamp, 0, sink, 0);

    g.run()
}
