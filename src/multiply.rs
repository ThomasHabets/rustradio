//! Multiply two streams.
use crate::Sample;
use crate::stream::{ReadStream, WriteStream};

/// Multiplies two streams, sample wise.
///
/// Output tags are taken from the first stream. Tags from the other input
/// stream is discarded.
///
/// ```
/// use rustradio::graph::{Graph, GraphRunner};
/// use rustradio::blocks::{ConstantSource, SignalSourceFloat, Multiply, NullSink};
///
/// let mut graph = Graph::new();
///
/// // Multiply a constant value.
/// let (src1, src1_out) = ConstantSource::new(1.0);
/// let (src2, src2_out) = SignalSourceFloat::new(44100.0, 1000.0, 1.0);
///
/// // Sum up the streams.
/// let (mul, mul_out) = Multiply::new(src1_out, src2_out);
///
/// graph.add(Box::new(src1));
/// graph.add(Box::new(src2));
/// graph.add(Box::new(mul));
///
/// // Set up dummy sink.
/// let sink = NullSink::new(mul_out);
/// # return Ok(());
/// graph.run()?;
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Multiply<Ta, Tb, Tout>
where
    Ta: Sample + std::ops::Mul<Tb, Output = Tout>,
    Tb: Sample,
    Tout: Sample,
{
    #[rustradio(in)]
    a: ReadStream<Ta>,

    #[rustradio(in)]
    b: ReadStream<Tb>,

    #[rustradio(out)]
    dst: WriteStream<Tout>,
}

impl<Ta, Tb, Tout> Multiply<Ta, Tb, Tout>
where
    Ta: Sample + std::ops::Mul<Tb, Output = Tout>,
    Tb: Sample,
    Tout: Sample,
{
    fn process_sync(&self, a: Ta, b: Tb) -> Tout {
        a * b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Float;
    use crate::block::Block;
    use crate::blocks::VectorSource;
    use crate::stream::{Tag, TagValue};

    #[test]
    fn mul_float() -> crate::Result<()> {
        // Testing VectorSource too, because why not.
        let input_a: Vec<_> = (0..10).map(|i| i as Float).collect();
        let (mut ablock, a) = VectorSource::new(input_a);
        ablock.work()?;

        let input_b: Vec<_> = (0..20).map(|i| 2.0 * (i as Float)).collect();
        let (mut bblock, b) = VectorSource::new(input_b);
        bblock.work()?;

        let (mut mul, os) = Multiply::new(a, b);
        mul.work()?;
        let (res, tags) = os.read_buf()?;
        let want: Vec<_> = (0..10).map(|i| i * (2 * i)).collect();
        let got: Vec<_> = res.slice().iter().map(|f| *f as usize).collect();
        assert_eq!(got, want);
        assert_eq!(
            tags,
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true))
            ]
        );
        Ok(())
    }
}
/* vim: textwidth=80
 */
