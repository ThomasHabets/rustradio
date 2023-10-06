#![warn(missing_docs)]
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
use rustradio::graph::Graph;
use rustradio::blocks::{AddConst, VectorSource, DebugSink};
use rustradio::stream::StreamType;
use rustradio::Complex;
let mut g = Graph::new();
let src = g.add(Box::new(VectorSource::new(
    vec![
        Complex::new(10.0, 0.0),
        Complex::new(-20.0, 0.0),
        Complex::new(100.0, -100.0),
    ],
    false,
)));
let add = g.add(Box::new(AddConst::new(Complex::new(1.1, 2.0))));
let sink = g.add(Box::new(DebugSink::<Complex>::new()));
g.connect(StreamType::new_complex(), src, 0, add, 0);
g.connect(StreamType::new_complex(), add, 0, sink, 0);
g.run()?;
# Ok::<(), anyhow::Error>(())
```

[sparslog]: https://github.com/ThomasHabets/sparslog
[gnuradio]: https://www.gnuradio.org/
 */
use anyhow::Result;

// Blocks.
pub mod add_const;
pub mod au;
pub mod binary_slicer;
pub mod complex_to_mag2;
pub mod constant_source;
pub mod convert;
pub mod debug_sink;
pub mod delay;
pub mod fft_filter;
pub mod file_sink;
pub mod file_source;
pub mod fir;
pub mod multiply_const;
pub mod null_sink;
pub mod quadrature_demod;
pub mod rational_resampler;
pub mod rtlsdr;
pub mod single_pole_iir_filter;
pub mod symbol_sync;
pub mod tcp_source;
pub mod vector_source;

#[cfg(feature = "rtlsdr")]
pub mod rtlsdr_source;

#[cfg(feature = "volk")]
pub mod volk;

pub mod block;
pub mod blocks;
pub mod graph;
pub mod stream;

/// Float type used. Usually f32, but not guaranteed.
pub type Float = f32;

/// Complex (I/Q) data.
pub type Complex = num::complex::Complex<Float>;

/// RustRadio error.
#[derive(Debug, Clone)]
pub struct Error {
    msg: String,
}

impl Error {
    /// Create new error with message.
    pub fn new(msg: &str) -> Self {
        Self {
            msg: msg.to_string(),
        }
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
