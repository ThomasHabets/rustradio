//! Blocks for making simple synchronous conversions/mappings.
use std::borrow::Cow;

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, Tag, WriteStream};
use crate::{Complex, Float, Result, Sample};

/// Like Map, but cannot modify what it sees.
///
/// ```
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::Inspect;
/// let (src_block, src) = ConstantSource::new(Complex::new(12.0, 13.0));
/// let (b, out) = Inspect::new(src, "mymap", move |x, _tags| println!("{x}"));
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(
    crate,
    custom_name,
    sync_tag,
    new,
    bound = "In: Sample, F: Fn(In, &[Tag]) + Send"
)]
pub struct Inspect<In, F> {
    #[rustradio(into)]
    name: String,
    f: F,
    #[rustradio(in)]
    src: ReadStream<In>,
    #[rustradio(out)]
    dst: WriteStream<In>,
}

impl<In, F> Inspect<In, F>
where
    In: Sample,
    F: Fn(In, &[Tag]) + Send,
{
    fn process_sync_tags<'a>(&mut self, s: In, tags: &'a [Tag]) -> (In, Cow<'a, [Tag]>) {
        (self.f)(s, tags);
        (s, Cow::Borrowed(tags))
    }
}
impl<In, F> Inspect<In, F> {
    /// Name of the block.
    pub fn custom_name(&self) -> &str {
        &self.name
    }
}

/// Block to convert from u8 to other sample types.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, bound = "T: Default+Clone")]
pub struct Parse<T> {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

impl<T: Sample<Type = T>> Block for Parse<T> {
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: make more efficient by doing batches.
        loop {
            // TODO: handle tags.
            let (i, _) = self.src.read_buf()?;
            if i.len() < T::size() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            let mut o = self.dst.write_buf()?;
            if o.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let s = T::parse(&i.slice()[..T::size()])?;
            o.slice()[0] = s;
            o.produce(1, &[]);
            i.consume(T::size());
        }
    }
}

/// Arbitrary mapping using a lambda.
///
/// A Map block transforms one sample at a time, from input to output. The input
/// and output can be different types.
///
/// If there's more than one input or output stream, then you have to make a
/// dedicated block, like [`FloatToComplex`].
///
/// ```
/// use std::borrow::Cow;
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::Map;
/// let (src_block, src) = ConstantSource::new(Complex::new(12.0, 13.0));
/// let (b, out) = Map::new(
///     src,
///     "mymap",
///     move |x, tags| (x.re + 10.0, Cow::Borrowed(tags)),
/// );
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(
    crate,
    custom_name,
    sync_tag,
    new,
    bound = "In: Sample, Out: Sample",
    bound = "F: for<'a> Fn(In, &'a [Tag]) -> (Out, Cow<'a, [Tag]>) + Send"
)]
pub struct Map<In, Out, F> {
    #[rustradio(into)]
    name: String,
    map: F,
    #[rustradio(in)]
    src: ReadStream<In>,
    #[rustradio(out)]
    dst: WriteStream<Out>,
}

#[allow(clippy::type_complexity)]
impl Map<(), (), ()> {
    /// Create a Map that just passes tags along.
    ///
    /// The specialization args (`AnySample` and the callback) are discarded,
    /// just to make `Map::keep_tags(src, "some name", |x| x * 2)` compile.
    #[allow(clippy::type_complexity)]
    pub fn keep_tags<In, Out, Name, F2>(
        src: ReadStream<In>,
        name: Name,
        f: F2,
    ) -> (
        Map<In, Out, impl for<'a> Fn(In, &'a [Tag]) -> (Out, Cow<'a, [Tag]>)>,
        ReadStream<Out>,
    )
    where
        In: Sample,
        Out: Sample,
        Name: Into<String>,
        F2: Fn(In) -> Out + Send,
    {
        Map::new(src, name, move |s, tags| (f(s), Cow::Borrowed(tags)))
    }
}

impl<In, Out, F> Map<In, Out, F>
where
    In: Sample,
    Out: Sample,
    F: for<'a> Fn(In, &'a [Tag]) -> (Out, Cow<'a, [Tag]>) + Send,
{
    fn process_sync_tags<'a>(&mut self, s: In, tags: &'a [Tag]) -> (Out, Cow<'a, [Tag]>) {
        (self.map)(s, tags)
    }
}

impl<In, Out, F> Map<In, Out, F> {
    /// Name of the block.
    pub fn custom_name(&self) -> &str {
        &self.name
    }
}

/// Arbitrary mapping of non-copy streams using a lambda.
///
/// A NCMap block transforms one input sample at a time into one or more
/// outputs. The input and output can be different types.
///
/// If there's more than one input or output stream, then you have to make a
/// dedicated block.
///
/// ```
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::NCMap;
/// // […]
/// # let (_, src) = rustradio::stream::new_nocopy_stream::<Vec<rustradio::Float>>();
/// let (b, out) = NCMap::new(src, "mymap", |mut x, tags| {
///   x[0] += 1.0;
///   vec![(x, tags)]
/// });
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(
    crate,
    custom_name,
    new,
    bound = "In: Send + Sync",
    bound = "Out: Send + Sync",
    bound = "F: Fn(In, Vec<Tag>) -> Vec<(Out, Vec<Tag>)> + Send"
)]
pub struct NCMap<In, Out, F> {
    #[rustradio(into)]
    name: String,
    map: F,
    #[rustradio(in)]
    src: NCReadStream<In>,
    #[rustradio(out)]
    dst: NCWriteStream<Out>,
}

impl<In, Out, F> Block for NCMap<In, Out, F>
where
    In: Send + Sync,
    Out: Send + Sync,
    F: Fn(In, Vec<Tag>) -> Vec<(Out, Vec<Tag>)> + Send,
{
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: handle tags.
        loop {
            let Some((x, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            // eprintln!("{tags:?}");
            for (packet, new_tags) in (self.map)(x, tags) {
                self.dst.push(packet, new_tags);
            }
        }
    }
}

impl<In, Out, F> NCMap<In, Out, F> {
    /// Name of the block.
    pub fn custom_name(&self) -> &str {
        &self.name
    }
}

/// Convert two floats stream to one complex stream.
///
/// ```
/// use rustradio::blocks::ConstantSource;
/// use rustradio::blocks::FloatToComplex;
/// let (re_block, re_src) = ConstantSource::new(1.0);
/// let (im_block, im_src) = ConstantSource::new(42.0);
/// let (b, out) = FloatToComplex::new(re_src, im_src);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct FloatToComplex {
    #[rustradio(in)]
    re: ReadStream<Float>,
    #[rustradio(in)]
    im: ReadStream<Float>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl FloatToComplex {
    fn process_sync(&self, re: Float, im: Float) -> Complex {
        Complex::new(re, im)
    }
}

/// Convert a complex stream to two float streams.
///
/// If only one of the two streams are needed, then it's better to use a [`Map`]
/// block.
///
/// ```
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::blocks::ComplexToFloat;
/// let (src_block, src) = ConstantSource::new(Complex::new(1.0, 2.0));
/// let (b, out_re, out_im) = ComplexToFloat::new(src);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct ComplexToFloat {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    re: WriteStream<Float>,
    #[rustradio(out)]
    im: WriteStream<Float>,
}

impl ComplexToFloat {
    fn process_sync(&self, c: Complex) -> (Float, Float) {
        (c.re, c.im)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSinkNoCopy;
    use crate::stream::{Tag, TagValue, new_nocopy_stream};

    #[test]
    fn ncmap_identity() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |packet, tags| vec![(packet, tags)]);
        let mut sink = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        tx.push(
            vec![9u8, 33, 22, 11],
            &[Tag::new(0, "foo", TagValue::U64(42))],
        );
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(
            &**r,
            vec![
                (vec![0u8, 1, 2, 3], vec![]),
                (
                    vec![9u8, 33, 22, 11],
                    vec![Tag::new(0, "foo", TagValue::U64(42))],
                ),
            ]
        );
        Ok(())
    }
    #[test]
    fn ncmap_drop() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |_packet, _tags| vec![]);
        let mut sink: VectorSinkNoCopy<Vec<u8>> = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(&**r, vec![]);
        Ok(())
    }
    #[test]
    fn ncmap_multipacket() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>, tags| {
            vec![
                (packet.iter().map(|s| *s * 2).collect(), tags.clone()),
                (packet.iter().map(|s| *s * 20).collect(), tags.clone()),
            ]
        });
        let mut sink = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(
            &**r,
            vec![
                (vec![0u8, 2, 4, 6], vec![]),
                (vec![0u8, 20, 40, 60], vec![]),
            ]
        );
        Ok(())
    }
    #[test]
    fn ncmap_double() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>, tags| {
            vec![(packet.iter().map(|s| *s * 2).collect(), tags)]
        });
        let mut sink = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(&**r, vec![(vec![0u8, 2, 4, 6], vec![])]);
        Ok(())
    }
    #[test]
    fn ncmap_double_inplace() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |mut packet: Vec<u8>, tags| {
            packet.iter_mut().for_each(|v| *v = *v + *v);
            vec![(packet, tags)]
        });
        let mut sink = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(&**r, vec![(vec![0u8, 2, 4, 6], vec![])]);
        Ok(())
    }
    #[test]
    fn ncmap_convert() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>, tags| {
            vec![(packet.iter().map(|s| *s as Float + 0.1).collect(), tags)]
        });
        let mut sink = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(&**r, vec![(vec![0.1 as Float, 1.1, 2.1, 3.1], vec![])]);
        Ok(())
    }
    #[test]
    fn ncmap_append() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>, tags| {
            let mut p2 = packet.clone();
            p2.extend(packet);
            vec![(p2, tags)]
        });
        let mut sink = VectorSinkNoCopy::new(out, 10);
        let res = sink.storage();
        tx.push(vec![0u8, 1, 2, 3], &[]);
        m.work()?;
        sink.work()?;
        let r = res.lock().unwrap();
        assert_eq!(&**r, vec![(vec![0u8, 1, 2, 3, 0, 1, 2, 3], vec![])]);
        Ok(())
    }
}
