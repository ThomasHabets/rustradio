//! Add a constant value to every sample.
use crate::Sample;
use crate::stream::{ReadStream, WriteStream};

/// Add const value, implemented in terms of Map.
///
/// This is basically example code. We have AddConst and add_const doing the
/// same thing.
pub fn add_const<T>(
    src: ReadStream<T>,
    val: T,
) -> (crate::convert::Map<T, T, impl Fn(T) -> T>, ReadStream<T>)
where
    T: Sample + std::ops::Add<Output = T>,
{
    crate::convert::Map::new(src, "add_const", move |x| x + val)
}

/// AddConst adds a constant value to every sample.
///
/// Tags are preserved.
///
/// ```
/// use rustradio::graph::{Graph, GraphRunner};
/// use rustradio::blocks::{ConstantSource, SignalSourceFloat, AddConst, NullSink};
///
/// let mut graph = Graph::new();
///
/// // Add a constant value. Could just as well use AddConst instead of Add.
/// let (src, src_out) = SignalSourceFloat::new(44100.0, 1000.0, 1.0);
///
/// // Sum up the streams.
/// let (sum, sum_out) = AddConst::new(src_out, 1.0);
///
/// graph.add(Box::new(src));
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
pub struct AddConst<T: Sample + std::ops::Add<Output = T>> {
    val: T,
    #[rustradio(in)]
    src: ReadStream<T>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T> AddConst<T>
where
    T: Sample + std::ops::Add<Output = T>,
{
    fn process_sync(&self, a: T) -> T {
        a + self.val
    }
}
