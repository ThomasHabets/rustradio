//! Blocks for making simple synchronous conversions/mappings.

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, WriteStream};
use crate::{Complex, Float, Result, Sample};

/// Like Map, but cannot modify what it sees.
///
/// ```
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::Inspect;
/// let (src_block, src) = ConstantSource::new(Complex::new(12.0, 13.0));
/// let (b, out) = Inspect::new(src, "mymap", move |x| println!("{x}"));
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, custom_name, sync, new)]
pub struct Inspect<In, F>
where
    In: Sample,
    F: Fn(In) + Send,
{
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
    F: Fn(In) + Send,
{
    fn process_sync(&mut self, s: In) -> In {
        (self.f)(s);
        s
    }
    /// Name of the block.
    pub fn custom_name(&self) -> &str {
        &self.name
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
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::Map;
/// let (src_block, src) = ConstantSource::new(Complex::new(12.0, 13.0));
/// let (b, out) = Map::new(src, "mymap", move |x| x.re + 10.0);
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, custom_name, sync, new)]
pub struct Map<In, Out, F>
where
    In: Sample,
    Out: Sample,
    F: Fn(In) -> Out + Send,
{
    #[rustradio(into)]
    name: String,
    map: F,
    #[rustradio(in)]
    src: ReadStream<In>,
    #[rustradio(out)]
    dst: WriteStream<F::Output>,
}

impl<In, Out, F> Map<In, Out, F>
where
    In: Sample,
    Out: Sample,
    F: Fn(In) -> Out + Send,
{
    fn process_sync(&mut self, s: In) -> Out {
        (self.map)(s)
    }
    /// Name of the block.
    pub fn custom_name(&self) -> &str {
        &self.name
    }
}

/// Arbitrary mapping of non-copy streams using a lambda.
///
/// A NCMap block transforms one sample at a time, from input to output. The
/// input and output can be different types.
///
/// If there's more than one input or output stream, then you have to make a
/// dedicated block.
///
/// ```text
/// use rustradio::Complex;
/// use rustradio::blocks::ConstantSource;
/// use rustradio::convert::NCMap;
/// […]
/// let (b, out) = NCMap::new(src, "mymap", |mut x| {
///   x[0] += 1;
///   Some(x)
/// });
/// ```
#[derive(rustradio_macros::Block)]
#[rustradio(crate, custom_name, new)]
pub struct NCMap<In, Out, F>
where
    In: Send + Sync,
    Out: Send + Sync,
    F: Fn(In) -> Option<Out> + Send,
{
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
    F: Fn(In) -> Option<Out> + Send,
{
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: handle tags.
        loop {
            let Some((x, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            eprintln!("{:?}", tags);
            if let Some(packet) = (self.map)(x) {
                self.dst.push(packet, &tags);
            }
        }
    }
}

impl<In, Out, F> NCMap<In, Out, F>
where
    In: Send + Sync,
    Out: Send + Sync,
    F: Fn(In) -> Option<Out> + Send,
{
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
        let (mut m, out) = NCMap::new(rx, "nctest", |packet| Some(packet));
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
                    vec![/*Tag::new(0, "foo", TagValue::U64(42))*/]
                ),
            ]
        );
        Ok(())
    }
    #[test]
    fn ncmap_drop() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |_packet| None);
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
    fn ncmap_double() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>| {
            Some(packet.iter().map(|s| *s * 2).collect())
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
        let (mut m, out) = NCMap::new(rx, "nctest", |mut packet: Vec<u8>| {
            packet.iter_mut().for_each(|v| *v = *v + *v);
            Some(packet)
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
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>| {
            Some(packet.iter().map(|s| *s as Float + 0.1).collect())
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
        let (mut m, out) = NCMap::new(rx, "nctest", |packet: Vec<u8>| {
            let mut p2 = packet.clone();
            p2.extend(packet);
            Some(p2)
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
