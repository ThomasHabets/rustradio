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
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

/*! This create provides a framework for running SDR (software defined
radio) applications.

It's heavily inspired by [GNURadio][gnuradio], except of course
written in Rust.

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

## Features

* `simd` (only with `nightly` Rust): Enable some code using `std::simd`.
* `rtlsdr`: Enable `RtlSdrSource` block, and adds the `rtlsdr` crate as a
  dependency at build time, and thus `librtlsdr.so` as a dependency at runtime.
* `soapysdr`: Add dependency on `soapysdr`, for its various SDR support.
* `fast-math`: Add a dependency in order to speed up some math.
* `audio`: Add support for `AudioSink`.
* `async`: Add support for `AsyncGraph`.
* `tokio-unstable`: For async graphs, allow use of tokio unstable API,

`tokio-unstable` allows tasks to be named, which helps when running
`tokio-console`. But it does require the user to run `cargo build` with the env
`RUSTFLAGS="--cfg tokio_unstable"` set too.

## Links

* Main repo: <https://github.com/ThomasHabets/rustradio>
* crates.io: <https://crates.io/crates/rustradio>
* This documentation: <https://docs.rs/rustradio/latest/rustradio/>

[sparslog]: https://github.com/ThomasHabets/sparslog
[gnuradio]: https://www.gnuradio.org/
 */
// Macro.
pub use rustradio_macros;

// Blocks.
pub mod add;
pub mod add_const;
pub mod au;
pub mod binary_slicer;
pub mod burst_tagger;
pub mod canary;
pub mod cma;
pub mod complex_to_mag2;
pub mod constant_source;
pub mod convert;
pub mod correlate_access_code;
pub mod debug_sink;
pub mod delay;
pub mod descrambler;
pub mod fft;
pub mod fft_filter;
pub mod fft_stream;
pub mod file_sink;
pub mod file_source;
pub mod fir;
pub mod hasher;
pub mod hdlc_deframer;
pub mod hdlc_framer;
pub mod hilbert;
pub mod iir_filter;
pub mod il2p_deframer;
pub mod kiss;
pub mod morse_encode;
pub mod multiply_const;
pub mod nrzi;
pub mod null_sink;
pub mod pdu_to_stream;
pub mod pdu_writer;
pub mod quadrature_demod;
pub mod rational_resampler;
pub mod reader_source;
pub mod rtlsdr_decode;
pub mod sigmf;
pub mod signal_source;
pub mod single_pole_iir_filter;
pub mod skip;
pub mod stream_to_pdu;
pub mod strobe;
pub mod symbol_sync;
pub mod tcp_source;
pub mod tee;
pub mod to_text;
pub mod vco;
pub mod vec_to_stream;
pub mod vector_sink;
pub mod vector_source;
pub mod wpcr;
pub mod writer_sink;
pub mod xor;
pub mod xor_const;
pub mod zero_crossing;

#[cfg(feature = "audio")]
pub mod audio_sink;

#[cfg(feature = "pipewire")]
pub mod pipewire_sink;

#[cfg(feature = "pipewire")]
pub mod pipewire_source;

#[cfg(feature = "rtlsdr")]
pub mod rtlsdr_source;

#[cfg(feature = "soapysdr")]
pub mod soapysdr_sink;

#[cfg(feature = "soapysdr")]
pub mod soapysdr_source;

pub mod block;
pub mod blocks;

#[cfg(not(feature = "wasm"))]
pub mod circular_buffer;

pub mod graph;
pub mod mtgraph;
pub mod stream;
pub mod window;

#[cfg(feature = "async")]
pub mod agraph;

/// Float type used. Usually f32, but not guaranteed.
pub type Float = f32;

/// Complex (I/Q) data.
pub type Complex = num_complex::Complex<Float>;

pub(crate) static NEXT_STREAM_ID: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(1);

/// RustRadio error.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// File error annotated with a specific path.
    #[error("IO Error on {path:?}: {source:?}")]
    FileIo {
        #[source]
        source: std::io::Error,
        path: std::path::PathBuf,
    },

    /// An error happened with a device such as SDR or audio device.
    #[error("DeviceError: {msg:?}: {source:?}")]
    DeviceError {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        msg: Option<String>,
    },

    /// An IO error without a known file associated.
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    /// An error with only a plain text message.
    #[error("An error occurred: {0}")]
    Plain(String),

    /// A wrapper around another error.
    #[error("{msg:?}: {source:?}")]
    Other {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
        msg: Option<String>,
    },
}

impl Error {
    /// Create error from message.
    #[must_use]
    pub fn msg<S: Into<String>>(msg: S) -> Self {
        Self::Plain(msg.into())
    }

    /// Wrap an IO error also including the path.
    #[must_use]
    pub fn file_io<P: Into<std::path::PathBuf>>(source: std::io::Error, path: P) -> Self {
        Self::FileIo {
            path: path.into(),
            source,
        }
    }

    /// Wrap another error into an `Error::Other`.
    ///
    /// The underlying error is provided, as well as optional extra context.
    #[must_use]
    pub fn wrap<S: Into<String>>(
        source: impl std::error::Error + Send + Sync + 'static,
        msg: S,
    ) -> Self {
        let msg = msg.into();
        Self::Other {
            source: Box::new(source),
            msg: if msg.is_empty() { None } else { Some(msg) },
        }
    }

    /// Wrap an error blaming some hardware or simulated hardware.
    ///
    /// The underlying error is provided, as well as optional extra context.
    #[must_use]
    pub fn device<S: Into<String>>(
        source: impl std::error::Error + Send + Sync + 'static,
        msg: S,
    ) -> Self {
        let msg = msg.into();
        Self::DeviceError {
            source: Box::new(source),
            msg: if msg.is_empty() { None } else { Some(msg) },
        }
    }
}

#[macro_export]
macro_rules! error_from {
    ($ctx:literal, $($err_ty:ty),* $(,)?) => {
        $(
            impl From<$err_ty> for Error {
                fn from(e: $err_ty) -> Self {
                    let s = if $ctx.is_empty() {
                        format!("{}", std::any::type_name::<$err_ty>())
                    } else {
                        format!("{} in {}", std::any::type_name::<$err_ty>(), $ctx)
                    };
                    Error::wrap(e, s)
                }
            }
        )*
    };
}

#[macro_export]
macro_rules! blockchain {
    ($g:expr, $prev:ident, $($cons:expr),* $(,)?) => {{
        $(
            let (block, $prev) = $cons;
            $g.add(Box::new(block));
            )*
            $prev
    }};
}

error_from!(
    "", // Can't attribute to a specific set of blocks.
    std::sync::mpsc::RecvError,
    std::sync::mpsc::TryRecvError,
    std::string::FromUtf8Error,
    std::array::TryFromSliceError,
    std::num::TryFromIntError,
);

pub type Result<T> = std::result::Result<T, Error>;

/// Repeat between zero and infinite times.
#[derive(Debug)]
pub struct Repeat {
    repeater: Repeater,
    count: u64,
}

impl Repeat {
    /// Repeat finite number of times. 0 Means not even once. 1 is default.
    #[must_use]
    pub fn finite(n: u64) -> Self {
        Self {
            repeater: Repeater::Finite(n),
            count: 0,
        }
    }

    /// Repeat infinite number of times.
    #[must_use]
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

impl Feature {
    #[must_use]
    fn new<S: Into<String>>(name: S, build: bool, detected: bool) -> Self {
        Self {
            name: name.into(),
            build,
            detected,
        }
    }
}

#[must_use]
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
        assumptions.push(Feature::new(
            "FMA",
            cfg!(target_feature = "fma"),
            is_x86_feature_detected!("fma"),
        ));
        assumptions.push(Feature::new(
            "SSE",
            cfg!(target_feature = "sse"),
            is_x86_feature_detected!("sse"),
        ));
        assumptions.push(Feature::new(
            "SSE3",
            cfg!(target_feature = "sse3"),
            is_x86_feature_detected!("sse3"),
        ));
        assumptions.push(Feature::new(
            "AVX",
            cfg!(target_feature = "avx"),
            is_x86_feature_detected!("avx"),
        ));
        assumptions.push(Feature::new(
            "AVX2",
            cfg!(target_feature = "avx2"),
            is_x86_feature_detected!("avx2"),
        ));
    }

    // TODO: ideally we don't duplicate this test here, but reuse it from the
    // top of the file.
    //
    // We check for feature `simd` here as a substitute for checking we're on
    // nightly, where the feature stuff is allowed.
    #[cfg(all(
        feature = "simd",
        any(target_arch = "riscv32", target_arch = "riscv64")
    ))]
    {
        assumptions.push(Feature::new(
            "Vector",
            cfg!(target_feature = "v"),
            std::arch::is_riscv_feature_detected!("v"),
        ));
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
        Err(Error::msg(format!("{errs:?}")))
    }
}

/// Parse verbosity like "error", "warn", …
///
/// For use with clap. E.g.:
///
/// ```rust
/// use rustradio::parse_verbosity;
/// #[derive(clap::Parser)]
/// struct Opt {
///     #[arg(long, value_parser=parse_verbosity)]
///     verbose: usize,
/// }
/// ```
pub fn parse_verbosity(in_s: &str) -> std::result::Result<usize, String> {
    use std::str::FromStr;
    log::Level::from_str(in_s)
        .map_err(|e| format!("{e}. Valid values are: error, warn, info, debug, trace"))
        .map(|v| v as usize - 1)
}

/// Parse frequencies like "100k", "2M", etc.
///
/// For use with clap. E.g.:
///
/// ```rust
/// use rustradio::parse_frequency;
/// #[derive(clap::Parser)]
/// struct Opt {
///     /// Frequency.
///     #[arg(long, value_parser=parse_frequency)]
///     freq: f64,
///     /// Sample rate.
///     #[arg(long, value_parser=parse_frequency, default_value_t = 300000.0)]
///     sample_rate: f64,
/// }
/// ```
pub fn parse_frequency(in_s: &str) -> std::result::Result<f64, String> {
    let s_binding;
    let s = if in_s.contains('_') {
        // Only create a copy if input actually contains underscores.
        s_binding = in_s.replace('_', "");
        s_binding.as_str()
    } else {
        in_s
    };
    let (nums, mul) = {
        let last = match s.chars().last() {
            None => return Err("empty string is not a frequency".into()),
            Some(ch) => ch.to_lowercase().next().unwrap(),
        };
        if s.len() > 1 {
            let rest = &s[..(s.len() - 1)];
            match last {
                'k' => (rest, 1_000.0),
                'm' => (rest, 1_000_000.0),
                'g' => (rest, 1_000_000_000.0),
                _ => (s, 1.0),
            }
        } else {
            (s, 1.0)
        }
    };
    Ok(nums.parse::<f64>().map_err(|e| {
        format!("Invalid number {in_s}: {e}. Has to be a float with optional k/mg suffix")
    })? * mul)
}

/// A trait all sample types must implement.
pub trait Sample: Copy + Default + Send + Sync + 'static {
    /// The type of the sample.
    type Type;

    /// The serialized size of one sample.
    #[must_use]
    fn size() -> usize;

    /// Parse one sample.
    fn parse(data: &[u8]) -> Result<Self::Type>;

    /// Serialize one sample.
    #[must_use]
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
        std::mem::size_of::<Self>()
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

/// Trivial trait for types that have .len().
#[allow(clippy::len_without_is_empty)]
pub trait Len {
    /// Get the length.
    #[must_use]
    fn len(&self) -> usize;
}
impl<T> Len for Vec<T> {
    fn len(&self) -> usize {
        self.len()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
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
            "\nleft: {left:?}\nright: {right:?}",
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).norm_sqr().sqrt();
            if dist > 0.001 {
                assert_eq!(
                    left[i], right[i],
                    "\nElement {i}:\nleft: {left:?}\nright: {right:?}",
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
            "\nleft: {left:?}\nright: {right:?}",
        );
        for i in 0..left.len() {
            let dist = (left[i] - right[i]).sqrt();
            if dist > 0.001 {
                assert_eq!(left[i], right[i], "\nleft: {left:?}\nright: {right:?}");
            }
        }
    }

    #[test]
    fn check_env() -> Result<()> {
        assert!(!environment_str(&check_environment()?).is_empty());
        Ok(())
    }

    #[test]
    fn error_wrap() {
        use std::error::Error as SysError;
        let e = Error::msg("foo");
        assert!(matches![e, Error::Plain(_)]);
        let _e2: &dyn std::error::Error = &e;
        let e_str = e.to_string();
        assert_eq!(e_str, "An error occurred: foo");
        let e3 = Error::wrap(e, "foo");
        assert!(matches![e3, Error::Other { source: _, msg: _ }]);
        let e4 = e3.source().unwrap();
        assert_eq!(e_str, e4.to_string());
        let e5 = e4.downcast_ref::<Error>().unwrap();
        assert!(matches![e5, Error::Plain(_)]);
    }

    #[test]
    fn frequency() {
        for (i, want) in &[
            ("", None),
            ("k", None),
            ("r", None),
            (".k", None),
            ("0", Some(0.0f64)),
            ("0.", Some(0.0f64)),
            ("0.0", Some(0.0f64)),
            (".3", Some(0.3f64)),
            (".3k", Some(300.0f64)),
            ("3.k", Some(3_000.0f64)),
            ("100", Some(100.0)),
            ("123k", Some(123_000.0)),
            ("123.78922K", Some(123_789.22)),
            ("321m", Some(321_000_000.0)),
            ("2.45g", Some(2_450_000_000.0)),
            ("100r", None),
            ("r100", None),
            ("10k0", None),
            ("100_000", Some(100_000.0)),
            ("_1_2_3._4_", Some(123.4)),
        ] {
            let got = parse_frequency(i);
            match (got, want) {
                (Err(_), None) => {}
                (Ok(got), None) => panic!("For {i} got {got}, want error"),
                (Err(e), Some(want)) => panic!("For {i} got error {e:?}, want {want}"),
                (Ok(got), Some(want)) if got == *want => {}
                (Ok(got), Some(want)) => panic!("For {i} got {got} want {want}"),
            }
        }
    }
}
