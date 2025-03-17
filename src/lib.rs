// Enable `std::simd` if feature simd is enabled.
#![cfg_attr(feature = "simd", feature(portable_simd))]
// Enable RISC-V arch detection, if on a RISC-V arch.
#![cfg_attr(
    all(
        feature = "simd",
        any(target_arch = "riscv32", target_arch = "riscv64")
    ),
    feature(stdarch_riscv_feature_detection)
)]

/*! This create provides a framework for running SDR (software defined
radio) applications.

It's heavily inspired by [GNURadio][gnuradio], except of course
written in Rust.

It currently has very few blocks, and is missing tags, and PDU
messages.

In addition to the example applications in this crate, there's also a
[sparslog][sparslog] project using this framework, that decodes IKEA
Sparsnäs electricity meter RF signals.

# Architecture overview

A RustRadio application consists of blocks that are connected by
unidirectional streams. Each block has zero or more input streams, and
zero or more output streams.

The signal flows through the blocks from "sources" (blocks without any
input streams) to "sinks" (blocks without any output streams.

These blocks and streams are called a "graph", like the mathematical
concept of graphs that have nodes and edges.

A block does something to its input(s), and passes the result to its
output(s).

A typical graph will be something like:

```text
  [ Raw radio source ]
           ↓
      [ Filtering ]
           ↓
      [ Resampling ]
           ↓
     [ Demodulation ]
           ↓
     [ Symbol Sync ]
           ↓
[ Packet assembly and save ]
```

Or concretely, for [sparslog][sparslog]:

```text
     [ RtlSdrSource ]
           ↓
  [ RtlSdrDecode to convert from ]
  [ own format to complex I/Q    ]
           ↓
     [ FftFilter ]
           ↓
      [ RationalResampler ]
           ↓
      [ QuadratureDemod ]
           ↓
  [ AddConst for frequency offset ]
           ↓
   [ ZeroCrossing symbol sync ]
           ↓
     [ Custom Sparsnäs decoder ]
     [ block in the binary,    ]
     [ not in the framework    ]
```

# Examples

Here's a simple example that creates a couple of blocks, connects them
with streams, and runs the graph.

```
use rustradio::graph::{Graph, GraphRunner};
use rustradio::blocks::{AddConst, VectorSource, DebugSink};
use rustradio::Complex;
let (src, prev) = VectorSource::new(
    vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ],
);
let (add, prev) = AddConst::new(prev, Complex::new(1.1, 2.0));
let sink = DebugSink::new(prev);
let mut g = Graph::new();
g.add(Box::new(src));
g.add(Box::new(add));
g.add(Box::new(sink));
g.run()?;
# Ok::<(), anyhow::Error>(())
```

## Links

* Main repo: <https://github.com/ThomasHabets/rustradio>
* crates.io: <https://crates.io/crates/rustradio>
* This documentation: <https://docs.rs/rustradio/latest/rustradio/>

[sparslog]: https://github.com/ThomasHabets/sparslog
[gnuradio]: https://www.gnuradio.org/
 */
use anyhow::Result;

// Macro.
pub use rustradio_macros;

// Blocks.
pub mod add;
pub mod add_const;
pub mod au;
pub mod binary_slicer;
pub mod burst_tagger;
pub mod cma;
pub mod complex_to_mag2;
pub mod constant_source;
pub mod convert;
pub mod correlate_access_code;
pub mod debug_sink;
pub mod delay;
pub mod descrambler;
pub mod fft_filter;
pub mod fft_stream;
pub mod file_sink;
pub mod file_source;
pub mod fir;
pub mod hasher;
pub mod hdlc_deframer;
pub mod hilbert;
pub mod iir_filter;
pub mod il2p_deframer;
pub mod multiply_const;
pub mod nrzi;
pub mod null_sink;
pub mod pdu_writer;
pub mod quadrature_demod;
pub mod rational_resampler;
pub mod rtlsdr_decode;
pub mod sigmf;
pub mod signal_source;
pub mod single_pole_iir_filter;
pub mod skip;
pub mod stream_to_pdu;
pub mod symbol_sync;
pub mod tcp_source;
pub mod tee;
pub mod to_text;
pub mod vec_to_stream;
pub mod vector_sink;
pub mod vector_source;
pub mod wpcr;
pub mod xor;
pub mod xor_const;
pub mod zero_crossing;

#[cfg(feature = "audio")]
pub mod audio_sink;

#[cfg(feature = "rtlsdr")]
pub mod rtlsdr_source;

#[cfg(feature = "soapysdr")]
pub mod soapysdr_sink;

#[cfg(feature = "soapysdr")]
pub mod soapysdr_source;

pub mod block;
pub mod blocks;
pub mod circular_buffer;
pub mod graph;
pub mod mtgraph;
pub mod stream;
pub mod window;

/// Float type used. Usually f32, but not guaranteed.
pub type Float = f32;

/// Complex (I/Q) data.
pub type Complex = num_complex::Complex<Float>;

/// RustRadio error.
#[derive(Debug, Clone)]
pub struct Error {
    msg: String,
}

impl Error {
    /// Create new error with message.
    // TODO: remove in favour of msg()
    pub fn new(msg: &str) -> Self {
        Self {
            msg: msg.to_string(),
        }
    }
    /// Create error from message.
    pub fn msg(msg: &str) -> Self {
        Self::new(msg)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "RustRadio Error: {}", self.msg)
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Error {
        Error::new(&format!("{}", e))
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(&format!("IO error: {}", e))
    }
}

/// Repeat between zero and infinite times.
#[derive(Debug)]
pub struct Repeat {
    repeater: Repeater,
    count: u64,
}

impl Repeat {
    /// Repeat finite number of times. 0 Means not even once. 1 is default.
    pub fn finite(n: u64) -> Self {
        Self {
            repeater: Repeater::Finite(n),
            count: 0,
        }
    }

    /// Repeat infinite number of times.
    pub fn infinite() -> Self {
        Self {
            repeater: Repeater::Infinite,
            count: 0,
        }
    }

    /// Register a repeat being done, and return true if we should continue.
    #[must_use]
    pub fn again(&mut self) -> bool {
        self.count += 1;
        match self.repeater {
            Repeater::Finite(n) => {
                self.repeater = Repeater::Finite(n - 1);
                n > 1
            }
            Repeater::Infinite => true,
        }
    }

    /// Return true if repeating is done.
    #[must_use]
    pub fn done(&self) -> bool {
        match self.repeater {
            Repeater::Finite(n) => n == 0,
            Repeater::Infinite => false,
        }
    }

    /// Return how many repeats have fully completed.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }
}

#[derive(Debug)]
enum Repeater {
    Finite(u64),
    Infinite,
}

pub struct Feature {
    name: String,
    build: bool,
    detected: bool,
}

pub fn environment_str(features: &[Feature]) -> String {
    let mut s = "Feature   Build Detected\n".to_string();
    for feature in features {
        s += &format!(
            "{:10} {:-5}    {:-5}\n",
            feature.name, feature.build, feature.detected
        );
    }
    s
}

pub fn check_environment() -> Result<Vec<Feature>> {
    #[allow(unused_mut)]
    let mut assumptions: Vec<Feature> = Vec::new();
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        assumptions.push(Feature {
            name: "FMA".to_string(),
            build: cfg!(target_feature = "fma"),
            detected: is_x86_feature_detected!("fma"),
        });
        assumptions.push(Feature {
            name: "SSE".to_string(),
            build: cfg!(target_feature = "sse"),
            detected: is_x86_feature_detected!("sse"),
        });
        assumptions.push(Feature {
            name: "SSE3".to_string(),
            build: cfg!(target_feature = "sse3"),
            detected: is_x86_feature_detected!("sse3"),
        });
        assumptions.push(Feature {
            name: "AVX".to_string(),
            build: cfg!(target_feature = "avx"),
            detected: is_x86_feature_detected!("avx"),
        });
        assumptions.push(Feature {
            name: "AVX2".to_string(),
            build: cfg!(target_feature = "avx2"),
            detected: is_x86_feature_detected!("avx2"),
        });
    }

    // TODO: ideally we don't duplicate this test here, but reuse it from the top of the file.
    #[cfg(all(
        feature = "simd",
        any(target_arch = "riscv32", target_arch = "riscv64")
    ))]
    {
        assumptions.push(Feature {
            name: "Vector".to_string(),
            build: cfg!(target_feature = "v"),
            detected: std::arch::is_riscv_feature_detected!("v"),
        });
    }

    let errs: Vec<_> = assumptions
        .iter()
        .filter_map(|f| {
            if f.build && !f.detected {
                Some(format!(
                    "Feature {} assumed by build flags but not detected",
                    f.name
                ))
            } else {
                None
            }
        })
        .collect();
    if errs.is_empty() {
        Ok(assumptions)
    } else {
        Err(Error::new(&format!("{:?}", errs)).into())
    }
}

/// A trait all sample types must implement.
pub trait Sample {
    /// The type of the sample.
    type Type;

    /// The serialized size of one sample.
    fn size() -> usize;

    /// Parse one sample.
    fn parse(data: &[u8]) -> Result<Self::Type>;

    /// Serialize one sample.
    fn serialize(&self) -> Vec<u8>;
}

impl Sample for Complex {
    type Type = Complex;
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Complex is wrong size");
        }
        let i = Float::from_le_bytes(data[0..Self::size() / 2].try_into()?);
        let q = Float::from_le_bytes(data[Self::size() / 2..].try_into()?);
        Ok(Complex::new(i, q))
    }
    fn serialize(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        ret.extend(Float::to_le_bytes(self.re));
        ret.extend(Float::to_le_bytes(self.im));
        ret
    }
}

impl Sample for num_complex::Complex<i32> {
    type Type = num_complex::Complex<i32>;
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Complex is wrong size");
        }
        let i = i32::from_le_bytes(data[0..Self::size() / 2].try_into()?);
        let q = i32::from_le_bytes(data[Self::size() / 2..].try_into()?);
        Ok(num_complex::Complex::new(i, q))
    }
    fn serialize(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        ret.extend(i32::to_le_bytes(self.re));
        ret.extend(i32::to_le_bytes(self.im));
        ret
    }
}

impl Sample for Float {
    type Type = Float;
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Float is wrong size");
        }
        Ok(Float::from_le_bytes(data[0..Self::size()].try_into()?))
    }
    fn serialize(&self) -> Vec<u8> {
        Float::to_le_bytes(*self).to_vec()
    }
}

impl Sample for u8 {
    type Type = u8;
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: u8 is wrong size");
        }
        Ok(data[0])
    }
    fn serialize(&self) -> Vec<u8> {
        vec![*self]
    }
}

impl Sample for u32 {
    type Type = u32;
    fn size() -> usize {
        4
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Float is wrong size");
        }
        Ok(u32::from_le_bytes(data[0..Self::size()].try_into()?))
    }
    fn serialize(&self) -> Vec<u8> {
        u32::to_le_bytes(*self).to_vec()
    }
}

impl Sample for i32 {
    type Type = i32;
    fn size() -> usize {
        4
    }
    fn parse(data: &[u8]) -> Result<Self::Type> {
        if data.len() != Self::size() {
            panic!("TODO: Float is wrong size");
        }
        Ok(i32::from_le_bytes(data[0..Self::size()].try_into()?))
    }
    fn serialize(&self) -> Vec<u8> {
        i32::to_le_bytes(*self).to_vec()
    }
}

impl Sample for String {
    type Type = String;
    fn size() -> usize {
        // TODO: variable.
        4
    }
    fn parse(_data: &[u8]) -> Result<Self::Type> {
        Ok("TODO".into())
    }
    fn serialize(&self) -> Vec<u8> {
        // TODO: there has to be a better way to do his. But I'm on a
        // plane with no wifi, so can't google it.
        let mut v = Vec::new();
        for ch in self.bytes() {
            v.push(ch);
        }
        v
    }
}

/// Trivial trait for types that have .len().
#[allow(clippy::len_without_is_empty)]
pub trait Len {
    /// Get the length.
    fn len(&self) -> usize;
}
impl<T> Len for Vec<T> {
    fn len(&self) -> usize {
        self.len()
    }
}

#[cfg(test)]
pub mod tests {
    //! Test helper functions.
    use super::*;

    /// For testing, assert that two slices are almost equal.
    ///
    /// Floating point numbers are almost never exactly equal.
    pub fn assert_almost_equal_complex(left: &[Complex], right: &[Complex]) {
        assert_eq!(
            left.len(),
            right.len(),
            "\nleft: {:?}\nright: {:?}",
            left,
            right
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).norm_sqr().sqrt();
            if dist > 0.001 {
                assert_eq!(
                    left[i], right[i],
                    "\nElement {i}:\nleft: {:?}\nright: {:?}",
                    left, right
                );
            }
        }
    }

    /// For testing, assert that two slices are almost equal.
    ///
    /// Floating point numbers are almost never exactly equal.
    pub fn assert_almost_equal_float(left: &[Float], right: &[Float]) {
        assert_eq!(
            left.len(),
            right.len(),
            "\nleft: {:?}\nright: {:?}",
            left,
            right
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).sqrt();
            if dist > 0.001 {
                assert_eq!(left[i], right[i], "\nleft: {:?}\nright: {:?}", left, right);
            }
        }
    }
}
