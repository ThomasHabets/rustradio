//use std::cell::RefCell;
//use std::collections::VecDeque;
use std::io::BufReader;
//use std::io::BufWriter;
use std::io::Read;
//use std::io::Write;
//use std::rc::Rc;
//use std::sync::Arc;
use futures_util::StreamExt;

use anyhow::Result;

pub type Float = f32;
pub type Complex = num_complex::Complex<Float>;

/*
struct RationalResampler<'a, T> {
    src: &'a mut dyn Iterator<Item = T>,
    obuf: VecDeque<T>,
    deci: i64,
    interp: i64,
    counter: i64,
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    a
}

impl<'a, T> RationalResampler<'a, T>
where
    T: Copy + Serial,
{
    pub fn new(
        src: &'a mut dyn Iterator<Item = T>,
        mut interp: usize,
        mut deci: usize,
    ) -> Result<Self> {
        let g = gcd(deci, interp);
        deci /= g;
        interp /= g;
        Ok(Self {
            src,
            obuf: VecDeque::new(),
            interp: i64::try_from(interp)?,
            deci: i64::try_from(deci)?,
            counter: 0,
        })
    }
}

impl<'a, T> Iterator for RationalResampler<'a, T>
where
    T: Copy + Serial,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.obuf.is_empty() {
            while self.obuf.is_empty() {
                let s = self.src.next()?;
                self.counter += self.interp;
                while self.counter >= 0 {
                    self.obuf.push_back(s);
                    self.counter -= self.deci;
                }
            }
        }
        self.obuf.pop_front()
    }
}

struct FftFilter<'a> {
    src: &'a mut dyn Iterator<Item = Complex>,

    obuf: VecDeque<Complex>,
    taps_fft: Vec<Complex>,
    nsamples: usize,
    fft_size: usize,
    tail: Vec<Complex>,
    fft: Arc<dyn rustfft::Fft<Float>>,
    ifft: Arc<dyn rustfft::Fft<Float>>,
}

impl<'a> FftFilter<'a> {
    fn calc_fft_size(from: usize) -> usize {
        let mut n = 1;
        while n < from {
            n <<= 1;
        }
        2 * n
    }
    fn new(src: &'a mut dyn Iterator<Item = Complex>, taps: &[Complex]) -> Self {
        // Set up FFT / batch size.
        let fft_size = Self::calc_fft_size(taps.len());
        let nsamples = fft_size - taps.len();

        // Create FFT planners.
        let mut planner = rustfft::FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let ifft = planner.plan_fft_inverse(fft_size);

        // Pre-FFT the taps.
        let mut taps_fft = taps.to_vec();
        taps_fft.resize(fft_size, Complex::default());
        fft.process(&mut taps_fft);

        // Normalization is actually the square root of this
        // expression, but since we'll do two FFTs we can just skip
        // the square root here and do it just once here in setup.
        {
            let f = 1.0 / taps_fft.len() as Float;
            taps_fft.iter_mut().for_each(|s: &mut Complex| *s *= f);
        }
        Self {
            src,
            obuf: VecDeque::new(),
            fft_size,
            taps_fft,
            tail: vec![Complex::default(); taps.len()],
            fft,
            ifft,
            nsamples,
        }
    }
}

impl<'a> Iterator for FftFilter<'a> {
    type Item = Complex;
    fn next(&mut self) -> Option<Complex> {
        // Flush output buffer.
        if let Some(x) = self.obuf.pop_front() {
            return Some(x);
        }
        // Fill input buffer.
        let mut buf = Vec::with_capacity(self.fft_size);

        while buf.len() < self.nsamples {
            buf.push(self.src.next()?);
        }

        // FFT.
        buf.resize(self.fft_size, Complex::default());
        self.fft.process(&mut buf);

        let mut filtered: Vec<Complex> = buf
            .iter()
            .zip(self.taps_fft.iter())
            .map(|(x, y)| x * y)
            .collect();

        // IFFT.
        self.ifft.process(&mut filtered);

        // Add overlapping tail.
        for (i, t) in self.tail.iter().enumerate() {
            filtered[i] += t;
        }

        // Output.
        for s in filtered[..self.nsamples].iter().copied() {
            self.obuf.push_back(s);
        }

        // Stash tail.
        for i in 0..self.tail.len() {
            self.tail[i] = filtered[self.nsamples + i];
        }
        self.obuf.pop_front()
    }
}

/// Create taps for a low pass filter as complex taps.
pub fn low_pass_complex(samp_rate: Float, cutoff: Float, twidth: Float) -> Vec<Complex> {
    low_pass(samp_rate, cutoff, twidth)
        .into_iter()
        .map(|t| Complex::new(t, 0.0))
        .collect()
}

/// Create taps for a low pass filter.
///
/// TODO: this could be faster if we supported filtering a Complex by a Float.
/// A low pass filter doesn't actually need complex taps.
pub fn low_pass(samp_rate: Float, cutoff: Float, twidth: Float) -> Vec<Float> {
    let pi = std::f64::consts::PI as Float;
    let ntaps = {
        let a: Float = 53.0; // Hamming.
        let t = (a * samp_rate / (22.0 * twidth)) as usize;
        if (t & 1) == 0 {
            t + 1
        } else {
            t
        }
    };
    let mut taps = vec![Float::default(); ntaps];
    let window: Vec<Float> = {
        // Hamming
        let m = (ntaps - 1) as Float;
        (0..ntaps)
            .map(|n| 0.54 - 0.46 * (2.0 * pi * (n as Float) / m).cos())
            .collect()
    };
    let m = (ntaps - 1) / 2;
    let fwt0 = 2.0 * pi * cutoff / samp_rate;
    for nm in 0..ntaps {
        let n = nm as i64 - m as i64;
        let nf = n as Float;
        taps[nm] = if n == 0 {
            fwt0 / pi * window[nm]
        } else {
            ((nf * fwt0).sin() / (nf * pi)) * window[nm]
        };
    }
    let gain = {
        let gain: Float = 1.0;
        let mut fmax = taps[m];
        for n in 1..=m {
            fmax += 2.0 * taps[n + m];
        }
        gain / fmax
    };
    taps.into_iter().map(|t| t * gain).collect()
}
*/
struct FileSource {
    f: BufReader<std::fs::File>,
}

impl FileSource {
    fn new(filename: &str) -> Result<Self> {
        Ok(Self {
            f: BufReader::new(std::fs::File::open(filename)?),
        })
    }
}

use futures_util::stream::Stream;
impl Stream for FileSource {
    type Item = Complex;
    fn poll_next(mut self: std::pin::Pin<&mut Self>, _cx: &mut std::task::Context<'_>)
                 -> std::task::Poll<Option<Self::Item>> {
        eprintln!("FileSource::poll_next");
        let mut buf = vec![0u8; 8];
        let n = self.f.read(&mut buf[..]).expect("failed to read");
        if n == 0 {
            std::task::Poll::Ready(None)
        } else {
            assert_eq!(n, 8);
            let c = Complex::new(
                Float::from_le_bytes(buf[0..4].try_into().expect("failed to parse")),
                Float::from_le_bytes(buf[4..8].try_into().expect("failed to parse")));
            std::task::Poll::Ready(Some(c))
        }
    }
}
/*
struct FileSink<'a, T> {
    src: &'a mut dyn Iterator<Item = T>,
    f: BufWriter<std::fs::File>,
}

impl<'a, T> FileSink<'a, T>
where
    T: Copy + Serial,
{
    fn new(src: &'a mut dyn Iterator<Item = T>, filename: &str) -> Result<Self> {
        Ok(Self {
            src,
            f: BufWriter::new(std::fs::File::create(filename)?),
        })
    }
}

impl<'a, T> Iterator for FileSink<'a, T>
where
    T: Copy + Serial,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        for v in &mut *self.src {
            self.f.write_all(&v.serial()).ok()?;
        }
        None
    }
}
*/
trait Serial {
    fn serial(&self) -> Vec<u8>;
}

impl Serial for Float {
    fn serial(&self) -> Vec<u8> {
        todo!()
    }
}

impl Serial for Complex {
    fn serial(&self) -> Vec<u8> {
        todo!()
    }
}

impl Serial for u8 {
    fn serial(&self) -> Vec<u8> {
        vec![*self]
    }
}
/*
struct QuadDemod<'a> {
    src: &'a mut dyn Iterator<Item = Complex>,
    last: Complex,
    gain: Float,
}

impl<'a> QuadDemod<'a> {
    fn new(src: &'a mut dyn Iterator<Item = Complex>, gain: Float) -> Self {
        Self {
            src,
            last: Complex::default(),
            gain,
        }
    }
}

impl<'a> Iterator for QuadDemod<'a> {
    type Item = Float;
    fn next(&mut self) -> Option<Float> {
        let s = self.src.next()?;
        let t = s * self.last.conj();
        self.last = s;
        //Some(self.gain * t.im.atan2(t.re))
        Some(self.gain * fast_math::atan2(t.im, t.re))
    }
}
*/
struct ConstantSource<T> {
    val: T,
}

impl<T> ConstantSource<T>
where
    T: Copy,
{
    fn new(val: T) -> Self {
        Self { val }
    }
}

impl<T> Stream for ConstantSource<T>
where
    T: Copy,
{
    type Item = T;
    fn poll_next(self: std::pin::Pin<&mut Self>, _cx: &mut std::task::Context<'_>)
                 -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(Some(self.val))
    }    
}


struct AddConst<T> {
    src: Pin<Box<dyn Stream<Item = T>>>,
    val: T,
}

impl<T> AddConst<T>
where
    T: Copy + Serial,
{
    fn new(src: Pin<Box<dyn Stream<Item = T>>>, val: T) -> Self { Self { src,val } }
//    fn new(src: Pin<Box<dyn Stream<Item = T>>>, val: T) -> Self { Self { src } }
}

impl<T> Stream for AddConst<T>
where
    T: Copy + std::ops::Add<Output = T> + Serial,
{
    type Item = T;
    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>)
                 -> Poll<Option<Self::Item>> {
        //match Pin::new(&mut self.src).as_mut().poll_next(cx) {
        match self.src.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(v)) => {
                std::task::Poll::Ready(Some(v))
            },
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        } 
    }
}
/*
struct MulConst<'a, T> {
    src: &'a mut dyn Iterator<Item = T>,
    val: T,
}

impl<'a, T> MulConst<'a, T>
where
    T: Copy + Serial,
{
    fn new(src: &'a mut dyn Iterator<Item = T>, val: T) -> Self {
        Self { src, val }
    }
}

impl<'a, T> Iterator for MulConst<'a, T>
where
    T: Copy + Serial + std::ops::Mul<Output = T>,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.src.next().map(|v| v * self.val)
    }
}

struct FloatToComplex<'a> {
    src1: &'a mut dyn Iterator<Item = Float>,
    src2: &'a mut dyn Iterator<Item = Float>,
}

impl<'a> FloatToComplex<'a> {
    fn new(
        src1: &'a mut dyn Iterator<Item = Float>,
        src2: &'a mut dyn Iterator<Item = Float>,
    ) -> Self {
        Self { src1, src2 }
    }
}

impl<'a> Iterator for FloatToComplex<'a> {
    type Item = Complex;
    fn next(&mut self) -> Option<Complex> {
        let a = self.src1.next()?;
        let b = self.src2.next()?;
        Some(Complex::new(a, b))
    }
}

struct ComplexToReal<'a> {
    src: &'a mut dyn Iterator<Item = Complex>,
}

impl<'a> ComplexToReal<'a> {
    fn new(src: &'a mut dyn Iterator<Item = Complex>) -> Self {
        Self { src }
    }
}

impl<'a> Iterator for ComplexToReal<'a> {
    type Item = Float;
    fn next(&mut self) -> Option<Float> {
        Some(self.src.next()?.re)
    }
}

struct AuEncode<'a> {
    obuf: VecDeque<u8>,
    src: &'a mut dyn Iterator<Item = Float>,
}

impl<'a> AuEncode<'a> {
    fn new(src: &'a mut dyn Iterator<Item = Float>, bitrate: u32, channels: u32) -> Self {
        let mut header = Vec::with_capacity(28);
        header.extend(0x2e736e64u32.to_be_bytes());
        header.extend(28u32.to_be_bytes());
        header.extend(0xffffffffu32.to_be_bytes());
        header.extend(3u32.to_be_bytes());
        header.extend(bitrate.to_be_bytes());
        header.extend(channels.to_be_bytes());
        header.extend(&[0, 0, 0, 0]);

        Self {
            src,
            obuf: header.into(),
        }
    }
}

impl<'a> Iterator for AuEncode<'a> {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        loop {
            if let Some(v) = self.obuf.pop_front() {
                return Some(v);
            }
            type S = i16;
            let scale = S::MAX as Float;

            let s = self.src.next()?;
            self.obuf.extend(((s * scale) as S).to_be_bytes());
        }
    }
}

struct Tee<'a, T> {
    pub src: &'a mut dyn Iterator<Item = T>,
    for_left: VecDeque<T>,
    for_right: VecDeque<T>,
}

struct TeePipe<'a, T> {
    left: bool,
    parent: Rc<RefCell<Tee<'a, T>>>,
}

impl<'a, T> Tee<'a, T>
where
    T: Copy + Serial,
{
    fn tee(src: &'a mut dyn Iterator<Item = T>) -> (TeePipe<'a, T>, TeePipe<'a, T>) {
        let t = Rc::new(RefCell::new(Self {
            src,
            for_left: VecDeque::new(),
            for_right: VecDeque::new(),
        }));
        (
            TeePipe::<T> {
                parent: t.clone(),
                left: true,
            },
            TeePipe::<T> {
                parent: t.clone(),
                left: false,
            },
        )
    }
}

impl<'a, T> Iterator for TeePipe<'a, T>
where
    T: Copy + Serial,
{
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.left {
            let mut m = self.parent.borrow_mut();
            if !m.for_left.is_empty() {
                return m.for_left.pop_front();
            }
            let ret = m.src.next()?;
            m.for_right.push_back(ret);
            Some(ret)
        } else {
            let mut m = self.parent.borrow_mut();
            if !m.for_right.is_empty() {
                return m.for_right.pop_front();
            }
            let ret = m.src.next()?;
            m.for_left.push_back(ret);
            Some(ret)
        }
    }
}
 */
use std::pin::Pin;
use std::task::{Context, Poll};

struct DebugSink<T> {
    src: Pin<Box<dyn Stream<Item = T>>>,
}

impl<T> DebugSink<T>
where
    T: Copy + Serial,
{
    fn new(src: Pin<Box<dyn Stream<Item = T>>>) -> Self {        Self { src }    }
}

impl<T> Stream for DebugSink<T>
where
    T: std::fmt::Debug + Serial + Copy,
{
    type Item = ();
    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>)
                 -> std::task::Poll<Option<Self::Item>> {
        match self.src.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(v)) => {
                eprintln!("Got item: {v:?}");
                std::task::Poll::Ready(Some(()))
            },
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {

    if true {
        let src = ConstantSource::new(1.0);

        //let (mut tee1, mut tee2) = Tee::tee(&mut src);
        let mut sink = AddConst::new(Box::pin(src), 0.5);
        //let mut convert = FloatToComplex::new(&mut add, &mut tee2);
        //let mut sink = DebugSink::new(Box::pin(add));
        while let Some(_) = sink.next().await {}
    }

    if false {
        // Source.
        let src = FileSource::new("raw-1024k.c32")?;
        let mut debug = DebugSink::new(Box::pin(src));
        while let Some(_) = debug.next().await {
            
        }
        eprintln!("stream done");
/*
        // Filter
        let taps = low_pass_complex(1024000.0, 100_000.0, 1000.0);
        let mut block = FftFilter::new(&mut src, &taps);

        // Resample RF.
        let mut block = RationalResampler::new(&mut block, 200_000, 1_024_000)?;

        // Quad Demod.
        let mut block = QuadDemod::new(&mut block, 1.0);

        // Filter audio.
        let taps = low_pass_complex(200_000.0, 44_100.0, 500.0);
        let mut zeroes = ConstantSource::new(0.0);
        let mut f2c = FloatToComplex::new(&mut block, &mut zeroes);
        let mut filter = FftFilter::new(&mut f2c, &taps);
        let mut block = ComplexToReal::new(&mut filter);

        // Resample audio.
        let mut block = RationalResampler::new(&mut block, 48_000, 200_000)?;

        // Change volume.
        let mut block = MulConst::new(&mut block, 0.2);

        // Convert to .au.
        let mut block = AuEncode::new(&mut block, 48_000, 1);

        // Write file.
        let mut block = FileSink::new(&mut block, "test.au")?;

        // Run flowgraph.
        if let Some(v) = block.next() {
            panic!("sink should never produce {:?}", v);
        }
*/
    }
    Ok(())
}
