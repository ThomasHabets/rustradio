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
use rustradio::graph::{Graph, GraphRunner, GraphRunnerExt};
use rustradio::blocks::{AddConst, VectorSource, DebugSink};
use rustradio::Complex;
let (src, prev) = VectorSource::new(
    vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ],
)?;
let (add, prev) = AddConst::new(prev, Complex::new(1.1, 2.0))?;
let sink = DebugSink::new(prev);
let mut g = Graph::new();
g.add(src);
g.add(add);
g.add(sink);
g.run()?;
# Ok::<(), anyhow::Error>(())
```

## Features

* `async`: Add support for `AsyncGraph`.
* `audio`: Add audio capture and playback (adds dependency).
* `fast-math`: Add a dependency in order to speed up some math.
* `fftw`: GPL code only: Add support to use `libfftw` instead of `rustfft`.
* `pipewire`: Add PipeWire blocks and enable native PipeWire for audio capture
  and playback on Linux and BSD (includes `audio`; requires PipeWire development
  libraries).
* `rtlsdr`: Enable `RtlSdrSource` block, and adds the `rtlsdr` crate as a
* `simd` (only with `nightly` Rust): Enable some code using `std::simd`.
  dependency at build time, and thus `librtlsdr.so` as a dependency at runtime.
* `soapysdr`: Add dependency on `soapysdr`, for its various SDR support.
* `tokio-unstable`: For async graphs, allow use of tokio unstable API,
* `uhd`: GPL code only: Enable the shared `UhdDevice`, `UhdSource`, and
  `UhdSink` blocks.
* `volk`: Add dependency on the `volk` library, to speed up some inner loops.
* `wasm`: Enable very experimental WASM support.

"GPL code only" means that this triggers linking against GPL code. So if you
distribute a binary with these features enabled, it causes your whole binary to
fall under GPL. This is my understanding. I am not a lawyer. You are responsible
for your own software supply chain.

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
// Enable some normally pedantic clippies.
#![deny(clippy::needless_for_each)]
#![deny(clippy::explicit_iter_loop)]
#![deny(clippy::items_after_statements)]
#![deny(clippy::format_push_string)]
#![deny(clippy::return_self_not_must_use)]
// Macro.
pub use rustradio_macros;

// Blocks.
pub mod add;
pub mod add_const;
pub mod agc;
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
pub mod head;
pub mod hilbert;
pub mod iir_filter;
pub mod il2p_deframer;
pub mod iq_balance;
#[cfg(feature = "unstable")]
pub mod iq_stream;
pub mod kiss;
pub mod morse_encode;
pub mod multiply;
pub mod multiply_const;
pub mod nrzi;
pub mod null_sink;
pub mod oscillator;
pub mod pdu_average;
pub mod pdu_to_stream;
pub mod pdu_writer;
pub mod pwm_decoder;
pub mod pwm_encoder;
pub mod quadrature_demod;
pub mod rational_resampler;
pub mod reader_source;
pub mod rotator;
pub mod rtlsdr_decode;
pub mod rtlsdr_encode;
pub mod sigmf;
pub mod signal_source;
pub mod single_pole_iir_filter;
pub mod skip;
#[cfg(feature = "unstable")]
pub mod stream_align;
pub mod stream_chunks;
pub mod stream_to_pdu;
pub mod strobe;
pub mod symbol_sync;
pub mod tcp_source;
pub mod tee;
pub mod to_text;
pub mod vco;
pub mod vector_sink;
pub mod vector_source;
pub mod wpcr;
pub mod writer_sink;
pub mod xor;
pub mod xor_const;
pub mod zero_crossing;

#[cfg(feature = "audio")]
mod audio;
#[cfg(feature = "audio")]
pub mod audio_sink;
#[cfg(feature = "audio")]
pub mod audio_source;

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

/// Re-export uhd dep since it may be a prerelease.
#[cfg(feature = "uhd")]
pub use uhd;

#[cfg(feature = "uhd")]
pub mod uhd_device;

#[cfg(feature = "uhd")]
pub mod uhd_sink;

#[cfg(feature = "uhd")]
pub mod uhd_source;

pub mod block;
pub mod blocks;

#[cfg(not(feature = "wasm"))]
pub mod nowasm;

#[cfg(feature = "wasm")]
pub mod wasm;

pub mod sys {
    #[cfg(not(feature = "wasm"))]
    pub use super::nowasm::export::*;
    #[cfg(feature = "wasm")]
    pub use super::wasm::export::*;
}

pub mod graph;
#[cfg(not(feature = "wasm"))]
pub mod mtgraph;
pub mod stream;
mod stream_metadata;
mod stream_observer;
mod stream_registry;
mod stream_tags;
pub mod window;

#[cfg(feature = "async")]
pub mod agraph;

/// Float type used. Usually f32, but not guaranteed.
pub type Float = f32;

/// Complex (I/Q) data.
pub type Complex = num_complex::Complex<Float>;

/// Addition that permits algebraic compiler optimizations.
///
/// Like [`std::ops::Add`], the right operand and output may differ from `Self`.
/// Floating-point implementations permit reassociation and do not promise the
/// same rounding, signed zeros, or exceptional values as ordinary operators.
/// Implement only the arithmetic capabilities your type supports; a FIR needs
/// [`AlgebraicAdd`] and [`AlgebraicMul`], without requiring [`AlgebraicDiv`].
pub trait AlgebraicAdd<Rhs = Self> {
    /// Result of the addition.
    type Output;
    /// Add two values, allowing algebraic optimizations.
    #[must_use]
    fn algebraic_add(self, rhs: Rhs) -> Self::Output;
}

/// Multiplication that permits algebraic compiler optimizations.
///
/// Supports mixed operands such as a complex value and a real scalar. Floating
/// implementations have the same optimization caveats as [`AlgebraicAdd`].
///
/// ```
/// use rustradio::{AlgebraicMul, Complex};
/// let value = Complex::new(3.0, -4.0);
/// assert_eq!(value.algebraic_mul(2.0), Complex::new(6.0, -8.0));
/// // UFCS avoids the primitive float's same-named inherent method.
/// assert_eq!(AlgebraicMul::algebraic_mul(2.0f32, value), Complex::new(6.0, -8.0));
/// ```
pub trait AlgebraicMul<Rhs = Self> {
    /// Result of the multiplication.
    type Output;
    /// Multiply two values, allowing algebraic optimizations.
    #[must_use]
    fn algebraic_mul(self, rhs: Rhs) -> Self::Output;
}

/// Division that permits algebraic compiler optimizations.
///
/// Floating implementations have the same optimization caveats as
/// [`AlgebraicAdd`]. Complex division uses the squared magnitude of the divisor;
/// extreme finite magnitudes can overflow or underflow intermediate values.
pub trait AlgebraicDiv<Rhs = Self> {
    /// Result of the division.
    type Output;
    /// Divide two values, allowing algebraic optimizations.
    #[must_use]
    fn algebraic_div(self, rhs: Rhs) -> Self::Output;
}

macro_rules! impl_algebraic_float {
    ($float:ty) => {
        impl AlgebraicAdd for $float {
            type Output = Self;
            #[inline(always)]
            fn algebraic_add(self, rhs: Self) -> Self {
                <$float>::algebraic_add(self, rhs)
            }
        }
        impl AlgebraicMul for $float {
            type Output = Self;
            #[inline(always)]
            fn algebraic_mul(self, rhs: Self) -> Self {
                <$float>::algebraic_mul(self, rhs)
            }
        }
        impl AlgebraicDiv for $float {
            type Output = Self;
            #[inline(always)]
            fn algebraic_div(self, rhs: Self) -> Self {
                <$float>::algebraic_div(self, rhs)
            }
        }
        impl AlgebraicAdd for num_complex::Complex<$float> {
            type Output = Self;
            #[inline(always)]
            fn algebraic_add(self, rhs: Self) -> Self {
                Self::new(self.re.algebraic_add(rhs.re), self.im.algebraic_add(rhs.im))
            }
        }
        impl AlgebraicMul for num_complex::Complex<$float> {
            type Output = Self;
            #[inline(always)]
            fn algebraic_mul(self, rhs: Self) -> Self {
                Self::new(
                    self.re
                        .algebraic_mul(rhs.re)
                        .algebraic_sub(self.im.algebraic_mul(rhs.im)),
                    self.re
                        .algebraic_mul(rhs.im)
                        .algebraic_add(self.im.algebraic_mul(rhs.re)),
                )
            }
        }
        impl AlgebraicDiv for num_complex::Complex<$float> {
            type Output = Self;
            #[inline(always)]
            fn algebraic_div(self, rhs: Self) -> Self {
                let denom = rhs
                    .re
                    .algebraic_mul(rhs.re)
                    .algebraic_add(rhs.im.algebraic_mul(rhs.im));
                Self::new(
                    self.re
                        .algebraic_mul(rhs.re)
                        .algebraic_add(self.im.algebraic_mul(rhs.im))
                        .algebraic_div(denom),
                    self.im
                        .algebraic_mul(rhs.re)
                        .algebraic_sub(self.re.algebraic_mul(rhs.im))
                        .algebraic_div(denom),
                )
            }
        }
        impl AlgebraicAdd<$float> for num_complex::Complex<$float> {
            type Output = Self;
            #[inline(always)]
            fn algebraic_add(self, rhs: $float) -> Self {
                Self::new(self.re.algebraic_add(rhs), self.im)
            }
        }
        impl AlgebraicMul<$float> for num_complex::Complex<$float> {
            type Output = Self;
            #[inline(always)]
            fn algebraic_mul(self, rhs: $float) -> Self {
                Self::new(self.re.algebraic_mul(rhs), self.im.algebraic_mul(rhs))
            }
        }
        impl AlgebraicDiv<$float> for num_complex::Complex<$float> {
            type Output = Self;
            #[inline(always)]
            fn algebraic_div(self, rhs: $float) -> Self {
                Self::new(self.re.algebraic_div(rhs), self.im.algebraic_div(rhs))
            }
        }
        impl AlgebraicAdd<num_complex::Complex<$float>> for $float {
            type Output = num_complex::Complex<$float>;
            #[inline(always)]
            fn algebraic_add(self, rhs: Self::Output) -> Self::Output {
                Self::Output::new(self.algebraic_add(rhs.re), rhs.im)
            }
        }
        impl AlgebraicMul<num_complex::Complex<$float>> for $float {
            type Output = num_complex::Complex<$float>;
            #[inline(always)]
            fn algebraic_mul(self, rhs: Self::Output) -> Self::Output {
                rhs.algebraic_mul(self)
            }
        }
        impl AlgebraicDiv<num_complex::Complex<$float>> for $float {
            type Output = num_complex::Complex<$float>;
            #[inline(always)]
            fn algebraic_div(self, rhs: Self::Output) -> Self::Output {
                let denom = rhs
                    .re
                    .algebraic_mul(rhs.re)
                    .algebraic_add(rhs.im.algebraic_mul(rhs.im));
                Self::Output::new(
                    self.algebraic_mul(rhs.re).algebraic_div(denom),
                    self.algebraic_mul(-rhs.im).algebraic_div(denom),
                )
            }
        }
    };
}
impl_algebraic_float!(f32);
impl_algebraic_float!(f64);

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

/// Create default `From<T>` for `Error`, with optional extra context.
///
/// ## Example
///
/// ```text
/// use rustradio::error_from;
/// error_from!(
///     "audio",
///     cpal::Error,
/// );
/// ```
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

/// Helper macro for creating a series of one-in, one-out blocks and adding them
/// to a graph.
///
/// ## Example
///
/// ```
/// use rustradio::graph::Graph;
/// use rustradio::blocks::*;
/// use rustradio::blockchain;
///
/// let mut g = Graph::new();
/// let prev = blockchain![
///     g,      // Graph object.
///     prev,   // Variable to use for the streams from block to block.
///     SignalSourceFloat::new(44100.0, 1000.0, 1.0)?,
///     MultiplyConst::new(prev, 2.0)?,
///     MultiplyConst::new(prev, 4.0)?,
/// ];
/// # Ok::<(), rustradio::Error>(())
/// ```
#[macro_export]
macro_rules! blockchain {
    ($g:expr, $prev:ident, $($cons:expr),* $(,)?) => {{
        use $crate::graph::GraphRunnerExt as _;
        $(
            let (block, $prev) = $cons;
            $g.add(block);
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

/// Result convenience type.
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
        self.count = self.count.saturating_add(1);
        match self.repeater {
            Repeater::Finite(0) => {
                log::error!(
                    "Repeat::again() called when repeat is 0, count {}",
                    self.count
                );
                false
            }
            Repeater::Finite(n) => self.count < n,
            Repeater::Infinite => true,
        }
    }

    /// Return true if repeating is done.
    #[must_use]
    pub fn done(&self) -> bool {
        match self.repeater {
            Repeater::Finite(n) => self.count >= n,
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

/// A CPU feature, such as `AVX` (x86) or `Vector` (RISC-V).
pub struct Feature {
    name: String,
    build: bool,
    detected: bool,
}

impl Feature {
    // Currently unused in WASM.
    #[must_use]
    #[allow(unused)]
    fn new<S: Into<String>>(name: S, build: bool, detected: bool) -> Self {
        Self {
            name: name.into(),
            build,
            detected,
        }
    }
}

/// Turn list of features into a nicely formatted table.
#[must_use]
pub fn environment_str(features: &[Feature]) -> String {
    use std::fmt::Write;
    let mut s = "Feature   Build Detected\n".to_string();
    for feature in features {
        let _ = writeln!(
            s,
            "{:10} {:-5}    {:-5}",
            feature.name, feature.build, feature.detected
        );
    }
    s
}

/// Check that the code wasn't built with features not detected at runtime.
///
/// This is a bigger problem than merely a runtime error would imply. If built
/// for features that are not here, that can only mean UB, before `main()` even
/// runs. So maybe this checking function doesn't actually add any value.
///
/// It does return the state of the features, though.
///
/// # Errors
///
/// If there are enabled processor features that are not supported. Though
/// again, this is UB.
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
///
/// # Errors
///
/// If the log level is not one of the known strings.
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
///
/// Supported features:
/// * k/m/g suffix (case insensitive).
/// * underscores are stripped.
///
/// # Errors
///
/// If frequency string is not of a valid form.
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
        let Some((last_idx, last)) = s.char_indices().next_back() else {
            return Err("empty string is not a frequency".into());
        };
        match last.to_ascii_lowercase() {
            'k' if last_idx > 0 => (&s[..last_idx], 1_000.0),
            'm' if last_idx > 0 => (&s[..last_idx], 1_000_000.0),
            'g' if last_idx > 0 => (&s[..last_idx], 1_000_000_000.0),
            _ => (s, 1.0),
        }
    };
    Ok(nums.parse::<f64>().map_err(|e| {
        format!("Invalid number {in_s}: {e}. Has to be a float with optional k/m/g suffix")
    })? * mul)
}

/// Parse durations like "10s", "1.5ms", or "2m 30s".
///
/// Units are `ns`, `us` (also `µs`), `ms`, `s`, `m`, `h`, `d`, and `w`.
/// Their full names, including plurals, are also accepted. Days are 24 hours
/// and weeks are seven days. Multiple components are added together, with
/// optional whitespace between numbers and units. Fractions are rounded to
/// the nearest nanosecond.
///
/// For use with clap:
///
/// ```rust
/// use rustradio::parse_duration;
/// #[derive(clap::Parser)]
/// struct Opt {
///     #[arg(long, value_parser=parse_duration, default_value="10s")]
///     interval: std::time::Duration,
/// }
/// ```
///
/// # Errors
///
/// If the input is empty, contains an invalid number or unit, is negative,
/// or exceeds [`std::time::Duration`]'s range. Every number needs a unit.
pub fn parse_duration(in_s: &str) -> std::result::Result<std::time::Duration, String> {
    use std::time::Duration;
    let mut remaining = in_s.trim();
    if remaining.is_empty() {
        return Err("empty string is not a duration".into());
    }
    let mut total = Duration::ZERO;
    while !remaining.is_empty() {
        let number_end = remaining
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(remaining.len());
        let number = &remaining[..number_end];
        let value = number
            .parse::<f64>()
            .map_err(|e| format!("Invalid duration number in {remaining:?}: {e}"))?;
        remaining = remaining[number_end..].trim_start();
        let unit_end = remaining
            .find(|c: char| !c.is_alphabetic())
            .unwrap_or(remaining.len());
        let unit = &remaining[..unit_end];
        let seconds = match unit {
            "ns" | "nanosecond" | "nanoseconds" => 1e-9,
            "us" | "µs" | "μs" | "microsecond" | "microseconds" => 1e-6,
            "ms" | "millisecond" | "milliseconds" => 1e-3,
            "s" | "sec" | "second" | "seconds" => 1.0,
            "m" | "min" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            "w" | "week" | "weeks" => 604800.0,
            _ => {
                return Err(format!(
                    "Invalid duration unit {unit:?}; use ns/us/ms/s/m/h/d/w"
                ));
            }
        };
        let component = Duration::try_from_secs_f64(value * seconds)
            .map_err(|e| format!("Invalid duration {number}{unit}: {e}"))?;
        total = total
            .checked_add(component)
            .ok_or_else(|| format!("Duration {in_s:?} exceeds the supported range"))?;
        remaining = remaining[unit_end..].trim_start();
    }
    Ok(total)
}

/// Parse an RFC3339 timestamp into [`std::time::SystemTime`].
///
/// Accepts a four-digit Gregorian year, a `T` separator, seconds, optional
/// fractional seconds, and a required `Z` or `±HH:MM` UTC offset. `T` and `Z`
/// are case insensitive. Fractional digits beyond nanoseconds are truncated.
/// Leap seconds (`:60`) map to the following second; announced leap-second
/// dates are not checked.
///
/// ```rust
/// use rustradio::parse_timestamp;
/// use std::time::{Duration, UNIX_EPOCH};
/// assert_eq!(parse_timestamp("1970-01-01T00:00:01Z")?,
///            UNIX_EPOCH + Duration::from_secs(1));
/// # Ok::<(), String>(())
/// ```
///
/// # Errors
///
/// If the syntax, date, time, or UTC offset is invalid, or the timestamp
/// cannot be represented by the platform's [`std::time::SystemTime`].
pub fn parse_timestamp(in_s: &str) -> std::result::Result<std::time::SystemTime, String> {
    use std::time::{Duration, UNIX_EPOCH};

    let invalid = || format!("Invalid RFC3339 timestamp {in_s:?}");
    let bytes = in_s.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(invalid());
    }
    let number = |digits: &[u8]| -> std::result::Result<i64, String> {
        digits.iter().try_fold(0, |n, &digit| {
            if digit.is_ascii_digit() {
                Ok(n * 10 + i64::from(digit - b'0'))
            } else {
                Err(invalid())
            }
        })
    };
    let year = number(&bytes[..4])?;
    let month = number(&bytes[5..7])?;
    let day = number(&bytes[8..10])?;
    let hour = number(&bytes[11..13])?;
    let minute = number(&bytes[14..16])?;
    let second = number(&bytes[17..19])?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || !(1..=month_days[(month - 1) as usize]).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return Err(invalid());
    }

    let mut pos = 19;
    let mut nanos = 0;
    if bytes[pos] == b'.' {
        pos += 1;
        let start = pos;
        let mut place = 100_000_000;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            nanos += u32::from(bytes[pos] - b'0') * place;
            place /= 10;
            pos += 1;
        }
        if pos == start {
            return Err(invalid());
        }
    }
    let offset = match &bytes[pos..] {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let hours = number(&[*h1, *h2])?;
            let minutes = number(&[*m1, *m2])?;
            if hours > 23 || minutes > 59 {
                return Err(invalid());
            }
            (hours * 3600 + minutes * 60) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return Err(invalid()),
    };

    // Count Gregorian days before this year, relative to 1970-01-01.
    // Euclidean division also handles the year 0000's preceding year.
    let preceding_year = year - 1;
    let days = preceding_year * 365 + preceding_year.div_euclid(4) - preceding_year.div_euclid(100)
        + preceding_year.div_euclid(400)
        - 719_162
        + month_days[..(month - 1) as usize].iter().sum::<i64>()
        + day
        - 1;
    let seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset;
    // Add the nonnegative fractional part after applying signed whole seconds,
    // so fractions before the Unix epoch still move forward within the second.
    let time = if seconds < 0 {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    } else {
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
    };
    time.and_then(|t| t.checked_add(Duration::from_nanos(u64::from(nanos))))
        .ok_or_else(|| format!("Timestamp {in_s:?} exceeds the platform's supported range"))
}

/// An in-memory sample that can be carried by processing blocks.
///
/// Automatically implemented for types satisfying the storage and threading
/// requirements. `Default` provides valid initial storage; it need not be
/// numerical zero. Processing a sample does not require a serialized format.
/// Byte-oriented sources and sinks additionally require [`SampleEncoding`].
pub trait Sample: std::fmt::Debug + Copy + Default + Send + Sync + 'static {}

impl<T: std::fmt::Debug + Copy + Default + Send + Sync + 'static> Sample for T {}

/// A sample's fixed-width byte encoding, independent of its in-memory layout.
///
/// Built-in encodings use little-endian bytes; complex samples encode the real
/// component followed by the imaginary component. Custom sample types only
/// need this trait when used with byte-oriented sources or sinks. Other byte
/// orders and formats can be represented by sample newtypes.
///
/// ```compile_fail,E0080
/// use rustradio::{Result, SampleEncoding};
/// use std::num::NonZeroUsize;
/// #[derive(Debug, Default, Copy, Clone)]
/// struct Bad(u8);
/// impl SampleEncoding for Bad {
///     const ENCODED_SIZE: NonZeroUsize = NonZeroUsize::new(0).unwrap();
///     fn parse(_: &[u8]) -> Result<Self> { Ok(Self(0)) }
///     fn serialize_into(&self, _: &mut Vec<u8>) {}
/// }
/// let _ = Bad::ENCODED_SIZE;
/// ```
pub trait SampleEncoding: Sample {
    /// The fixed, nonzero number of encoded bytes per sample.
    const ENCODED_SIZE: std::num::NonZeroUsize;

    /// Parse one sample of this type.
    ///
    /// # Errors
    ///
    /// For the built-in encodings, it can only fail if the
    /// passed byte slice is of the wrong size.
    fn parse(data: &[u8]) -> Result<Self>;

    /// Serialize one sample.
    #[must_use]
    fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::ENCODED_SIZE.get());
        self.serialize_into(&mut out);
        out
    }

    /// Append one sample's serialized bytes to an existing buffer.
    ///
    /// Append exactly `Self::ENCODED_SIZE.get()` bytes, preserving bytes already
    /// in `out`. Implement this directly so sinks can reuse their buffer.
    fn serialize_into(&self, out: &mut Vec<u8>);
}

impl SampleEncoding for Complex {
    const ENCODED_SIZE: std::num::NonZeroUsize =
        std::num::NonZeroUsize::new(std::mem::size_of::<Self>()).unwrap();
    fn parse(data: &[u8]) -> Result<Self> {
        if data.len() != Self::ENCODED_SIZE.get() {
            return Err(Error::msg(format!(
                "Tried to parse Complex from {} bytes",
                data.len()
            )));
        }
        let i = Float::from_le_bytes(data[0..Self::ENCODED_SIZE.get() / 2].try_into()?);
        let q = Float::from_le_bytes(data[Self::ENCODED_SIZE.get() / 2..].try_into()?);
        Ok(Complex::new(i, q))
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.re.to_le_bytes());
        out.extend_from_slice(&self.im.to_le_bytes());
    }
}

impl SampleEncoding for Float {
    const ENCODED_SIZE: std::num::NonZeroUsize =
        std::num::NonZeroUsize::new(std::mem::size_of::<Self>()).unwrap();
    fn parse(data: &[u8]) -> Result<Self> {
        if data.len() != Self::ENCODED_SIZE.get() {
            return Err(Error::msg(format!(
                "Tried to parse Float from {} bytes",
                data.len()
            )));
        }
        Ok(Float::from_le_bytes(
            data[0..Self::ENCODED_SIZE.get()].try_into()?,
        ))
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

impl SampleEncoding for u8 {
    const ENCODED_SIZE: std::num::NonZeroUsize =
        std::num::NonZeroUsize::new(std::mem::size_of::<Self>()).unwrap();
    fn parse(data: &[u8]) -> Result<Self> {
        if data.len() != Self::ENCODED_SIZE.get() {
            return Err(Error::msg(format!(
                "Tried to parse u8 from {} bytes",
                data.len()
            )));
        }
        Ok(data[0])
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.push(*self);
    }
}

impl SampleEncoding for u32 {
    const ENCODED_SIZE: std::num::NonZeroUsize = std::num::NonZeroUsize::new(4).unwrap();
    fn parse(data: &[u8]) -> Result<Self> {
        if data.len() != Self::ENCODED_SIZE.get() {
            return Err(Error::msg(format!(
                "Tried to parse u32 from {} bytes",
                data.len()
            )));
        }
        Ok(u32::from_le_bytes(
            data[0..Self::ENCODED_SIZE.get()].try_into()?,
        ))
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

impl SampleEncoding for i32 {
    const ENCODED_SIZE: std::num::NonZeroUsize =
        std::num::NonZeroUsize::new(std::mem::size_of::<Self>()).unwrap();
    fn parse(data: &[u8]) -> Result<Self> {
        if data.len() != Self::ENCODED_SIZE.get() {
            return Err(Error::msg(format!(
                "Tried to parse i32 from {} bytes",
                data.len()
            )));
        }
        Ok(i32::from_le_bytes(
            data[0..Self::ENCODED_SIZE.get()].try_into()?,
        ))
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

/// Trivial trait for types that have `.len()`.
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
            let dist = (left[i] - right[i]).abs();
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
            (".", None),
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
            ("123kk", None),
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

    #[test]
    fn duration() {
        use std::time::Duration;
        for (input, expected) in [
            ("0s", Duration::ZERO),
            ("10s", Duration::from_secs(10)),
            ("1.5ms", Duration::from_micros(1500)),
            (".5s", Duration::from_millis(500)),
            ("1ns", Duration::from_nanos(1)),
            ("1us", Duration::from_micros(1)),
            ("1µs", Duration::from_micros(1)),
            ("1μs", Duration::from_micros(1)),
            ("2m30s", Duration::from_secs(150)),
            (
                " 2 minutes 500 milliseconds ",
                Duration::from_millis(120_500),
            ),
            ("1h 1d 1w", Duration::from_secs(3600 + 86400 + 604800)),
            ("1.000000001s", Duration::from_nanos(1_000_000_001)),
        ] {
            assert_eq!(parse_duration(input).unwrap(), expected, "{input:?}");
        }
    }

    #[test]
    fn invalid_durations() {
        for input in [
            "",
            " ",
            "s",
            "10",
            "-1s",
            "+1s",
            "NaNs",
            "infs",
            ".s",
            "1..5s",
            "10fortnights",
            "1s garbage",
            "1s -2s",
            "1s 2",
            "1e3s",
            "1秒",
            "99999999999999999999999999999999999999w",
            "10000000000000000000s 10000000000000000000s",
        ] {
            assert!(parse_duration(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn timestamp() {
        use std::time::{Duration, UNIX_EPOCH};
        for (input, seconds, nanos) in [
            ("1970-01-01T00:00:00Z", 0i64, 0),
            ("1970-01-01t00:00:01z", 1, 0),
            ("1970-01-01T01:30:00+01:30", 0, 0),
            ("1969-12-31T22:30:00-01:30", 0, 0),
            ("1970-01-01T00:00:00-00:00", 0, 0),
            ("1969-12-31T23:59:59.5Z", -1, 500_000_000),
            ("1970-01-01T00:00:00.123456789123Z", 0, 123_456_789),
            ("2000-02-29T00:00:00Z", 951_782_400, 0),
            ("1900-03-01T00:00:00Z", -2_203_891_200, 0),
            ("0000-01-01T00:00:00Z", -62_167_219_200, 0),
            ("9999-12-31T23:59:59Z", 253_402_300_799, 0),
            ("2016-12-31T23:59:60Z", 1_483_228_800, 0),
        ] {
            let expected = if seconds < 0 {
                UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
            } else {
                UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
            }
            .and_then(|time| time.checked_add(Duration::from_nanos(nanos)));
            match expected {
                Some(expected) => assert_eq!(parse_timestamp(input).unwrap(), expected, "{input}"),
                None => assert!(parse_timestamp(input).is_err(), "{input}"),
            }
        }
    }

    #[test]
    fn invalid_timestamps() {
        for input in [
            "",
            "invalid",
            "2026-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "2024-04-31T00:00:00Z",
            "2026-00-01T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-01-00T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-01-01T00:00:61Z",
            "2026-01-01T00:00:00",
            "2026-01-01T00:00:00.Z",
            "2026-01-01T00:00:00+24:00",
            "2026-01-01T00:00:00+00:60",
            "2026-01-01T00:00:00+0000",
            "2026-01-01T00:00:00Zjunk",
            "2026-01-01T00:00:00Z ",
            " 2026-01-01T00:00:00Z",
            "２０２６-01-01T00:00:00Z",
            "2026-01-01T00:00:0éZ",
        ] {
            assert!(parse_timestamp(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn repeat_test_infinite() {
        let mut r = Repeat::infinite();
        for n in 0..100 {
            assert_eq!(n, r.count());
            assert!(!r.done());
            assert!(r.again());
        }
    }

    #[test]
    fn repeat_test_none() {
        let mut r = Repeat::finite(0);
        assert_eq!(0, r.count());
        assert!(r.done());
        assert!(!r.again());
        assert_eq!(1, r.count());
        assert!(r.done());
        assert!(!r.again());
        assert_eq!(2, r.count());
    }

    #[test]
    fn repeat_test_one() {
        let mut r = Repeat::finite(1);
        assert_eq!(0, r.count());
        assert!(!r.done());

        assert!(!r.again());
        assert_eq!(1, r.count());
        assert!(r.done());

        assert!(!r.again());
        assert_eq!(2, r.count());
        assert!(r.done());
    }

    #[test]
    fn repeat_test_three() {
        let mut r = Repeat::finite(3);
        assert_eq!(0, r.count());
        assert!(!r.done());

        assert!(r.again());
        assert_eq!(1, r.count());
        assert!(!r.done());

        assert!(r.again());
        assert_eq!(2, r.count());
        assert!(!r.done());

        assert!(!r.again());
        assert_eq!(3, r.count());
        assert!(r.done());
    }

    #[test]
    fn repeat_does_not_wrap_at_maximum() {
        let mut r = Repeat {
            repeater: Repeater::Finite(u64::MAX),
            count: u64::MAX,
        };
        assert!(!r.again());
        assert_eq!(r.count(), u64::MAX);
    }
}
