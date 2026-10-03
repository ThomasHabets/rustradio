//! Exercise both generated sync paths with tags and output backpressure.
use std::borrow::Cow;

use rustradio::Result;
use rustradio::block::{Block, BlockRet};
use rustradio::stream::{ReadStream, Tag, TagValue, WriteStream, new_stream};

#[derive(rustradio_macros::Block)]
#[rustradio(new, sync)]
struct Plain {
    #[rustradio(in)]
    a: ReadStream<u32>,
    #[rustradio(in)]
    b: ReadStream<u32>,
    #[rustradio(out)]
    sum: WriteStream<u32>,
    #[rustradio(out)]
    difference: WriteStream<u32>,
}

impl Plain {
    fn process_sync(&self, a: u32, b: u32) -> (u32, u32) {
        (a + b, a - b)
    }
}

#[derive(rustradio_macros::Block)]
#[rustradio(new, sync_tag)]
struct Tagged {
    #[rustradio(in)]
    a: ReadStream<u32>,
    #[rustradio(in)]
    b: ReadStream<u32>,
    #[rustradio(out)]
    sum: WriteStream<u32>,
    #[rustradio(out)]
    difference: WriteStream<u32>,
}

impl Tagged {
    fn process_sync_tags<'a>(
        &self,
        a: u32,
        a_tags: &'a [Tag],
        b: u32,
        b_tags: &'a [Tag],
    ) -> (u32, Cow<'a, [Tag]>, u32, Cow<'a, [Tag]>) {
        assert!(a_tags.iter().chain(b_tags).all(|tag| tag.pos() == 0));
        let mut tags = b_tags.to_vec();
        // Generated tags must also work on samples without input tags.
        tags.push(tag(0, "generated"));
        (a + b, Cow::Borrowed(a_tags), a - b, Cow::Owned(tags))
    }
}

fn tag(pos: usize, key: &str) -> Tag {
    Tag::new(pos, key, TagValue::String(key.to_string()))
}

fn input(samples: &[u32], tags: &[Tag]) -> Result<ReadStream<u32>> {
    let (writer, reader) = new_stream();
    let mut window = writer.write_buf()?;
    window.fill_from_slice(samples);
    window.produce(samples.len(), tags);
    Ok(reader)
}

fn inputs() -> Result<(ReadStream<u32>, ReadStream<u32>)> {
    Ok((
        input(
            &[10, 20, 30, 40, 50, 60],
            &[tag(0, "a"), tag(0, "a2"), tag(2, "a3"), tag(5, "a4")],
        )?,
        input(
            &[1, 2, 3, 4, 5, 6, 7],
            &[
                tag(1, "b"),
                tag(2, "b2"),
                tag(5, "b3"),
                tag(6, "unconsumed"),
            ],
        )?,
    ))
}

fn fill_output(writer: &WriteStream<u32>) -> Result<usize> {
    let mut window = writer.write_buf()?;
    let n = window.len() - 2;
    window.slice()[..n].fill(0);
    window.produce(n, &[]);
    Ok(n)
}

fn check_batches(
    block: &mut impl Block,
    sum: ReadStream<u32>,
    difference: ReadStream<u32>,
    filler: usize,
    tagged: bool,
) -> Result<()> {
    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    let (window, tags) = sum.read_buf()?;
    assert_eq!(&window.slice()[filler..], &[11, 22]);
    assert_eq!(tags, [tag(filler, "a"), tag(filler, "a2")]);
    window.consume(filler + 2);
    let (window, tags) = difference.read_buf()?;
    assert_eq!(window.slice(), &[9, 18]);
    let expected = if tagged {
        vec![tag(0, "generated"), tag(1, "b"), tag(1, "generated")]
    } else {
        vec![tag(0, "a"), tag(0, "a2")]
    };
    assert_eq!(tags, expected);
    window.consume(2);

    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    let (window, tags) = sum.read_buf()?;
    assert_eq!(window.slice(), &[33, 44, 55, 66]);
    assert_eq!(tags, [tag(0, "a3"), tag(3, "a4")]);
    window.consume(4);
    let (window, tags) = difference.read_buf()?;
    assert_eq!(window.slice(), &[27, 36, 45, 54]);
    let expected = if tagged {
        vec![
            tag(0, "b2"),
            tag(0, "generated"),
            tag(1, "generated"),
            tag(2, "generated"),
            tag(3, "b3"),
            tag(3, "generated"),
        ]
    } else {
        vec![tag(0, "a3"), tag(3, "a4")]
    };
    assert_eq!(tags, expected);
    window.consume(4);
    Ok(())
}

#[test]
fn plain_sync_forwards_first_input_tags_across_batches() -> Result<()> {
    let (a, b) = inputs()?;
    let (mut block, sum, difference) = Plain::new(a, b);
    let filler = fill_output(&block.sum)?;
    check_batches(&mut block, sum, difference, filler, false)?;
    let (remaining, tags) = block.b.read_buf()?;
    assert_eq!(remaining.slice(), &[7]);
    assert_eq!(tags, [tag(0, "unconsumed")]);
    Ok(())
}

#[test]
fn tagged_sync_groups_each_input_and_preserves_output_tag_order() -> Result<()> {
    let (a, b) = inputs()?;
    let (mut block, sum, difference) = Tagged::new(a, b);
    let filler = fill_output(&block.sum)?;
    check_batches(&mut block, sum, difference, filler, true)
}

#[test]
fn tagged_sync_can_generate_tags_without_input_tags() -> Result<()> {
    let a = input(&[10, 20, 30], &[])?;
    let b = input(&[1, 2, 3], &[])?;
    let (mut block, sum, difference) = Tagged::new(a, b);
    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    let (window, tags) = sum.read_buf()?;
    assert_eq!(window.slice(), &[11, 22, 33]);
    assert!(tags.is_empty());
    let (window, tags) = difference.read_buf()?;
    assert_eq!(window.slice(), &[9, 18, 27]);
    assert_eq!(
        tags,
        [
            tag(0, "generated"),
            tag(1, "generated"),
            tag(2, "generated")
        ]
    );
    Ok(())
}

#[test]
fn plain_sync_discards_tags_from_other_inputs() -> Result<()> {
    let a = input(&[10, 20], &[])?;
    let b = input(&[1, 2], &[tag(0, "discard"), tag(1, "discard")])?;
    let (mut block, sum, difference) = Plain::new(a, b);
    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    assert!(sum.read_buf()?.1.is_empty());
    assert!(difference.read_buf()?.1.is_empty());
    Ok(())
}
