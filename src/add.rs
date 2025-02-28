//! Add two streams.
//!
//! To add a constant value to a stream, instead use AddConst.
use crate::stream::{ReadStream, WriteStream};

/// Adds two streams, sample wise.
///
/// Output tags are taken from the first stream. Tags from the other input
/// stream is discarded.
///
/// To add a constant value to a stream, instead use AddConst.
///
/// ```
/// use rustradio::graph::{Graph, GraphRunner};
/// use rustradio::blocks::{ConstantSource, SignalSourceFloat, Add, NullSink};
///
/// let mut graph = Graph::new();
///
/// // Add a constant value. Could just as well use AddConst instead of Add.
/// let (src1, src1_out) = ConstantSource::new(1.0);
/// let (src2, src2_out) = SignalSourceFloat::new(44100.0, 1000.0, 1.0);
///
/// // Sum up the streams.
/// let (sum, sum_out) = Add::new(src1_out, src2_out);
///
/// graph.add(Box::new(src1));
/// graph.add(Box::new(src2));
/// graph.add(Box::new(sum));
///
/// // Set up dummy sink.
/// let sink = NullSink::new(sum_out);
/// # return Ok(());
/// graph.run()?;
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct Add<Ta, Tb, Tout>
where
    Ta: Copy + std::ops::Add<Tb, Output = Tout>,
    Tb: Copy,
    Tout: Copy,
{
    /// Hello world.
    #[rustradio(in)]
    a: ReadStream<Ta>,

    #[rustradio(in)]
    b: ReadStream<Tb>,

    #[rustradio(out)]
    dst: WriteStream<Tout>,
}

impl<Ta, Tb, Tout> Add<Ta, Tb, Tout>
where
    Ta: Copy + std::ops::Add<Tb, Output = Tout>,
    Tb: Copy,
    Tout: Copy,
{
    fn process_sync(&self, a: Ta, b: Tb) -> Tout {
        a + b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Float;
    use crate::block::Block;
    use crate::blocks::VectorSource;

    #[test]
    fn add_float() -> crate::Result<()> {
        // Testing VectorSource too, because why not.
        let input_a: Vec<_> = (0..10).map(|i| i as Float).collect();
        let (mut ablock, a) = VectorSource::new(input_a);
        ablock.work()?;

        let input_b: Vec<_> = (0..20).map(|i| 2.0 * (i as Float)).collect();
        let (mut bblock, b) = VectorSource::new(input_b);
        bblock.work()?;

        let (mut add, os) = Add::new(a, b);
        add.work()?;
        let (res, _) = os.read_buf()?;
        let want: Vec<_> = (0..10).map(|i| 3 * i).collect();
        let got: Vec<_> = res.slice().iter().map(|f| *f as usize).collect();
        assert_eq!(got, want);
        Ok(())
    }
}
/* vim: textwidth=80
 */
