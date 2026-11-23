//! Exercise both generated sync paths with tags and output backpressure.

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
    fn process_sync_tags(
        &self,
        a: u32,
        a_tags: Vec<Tag>,
        b: u32,
        mut b_tags: Vec<Tag>,
    ) -> (u32, Vec<Tag>, u32, Vec<Tag>) {
        assert!(
            a_tags
                .iter()
                .chain(b_tags.as_slice())
                .all(|tag| tag.pos() == 0)
        );

        // Generated tags must also work on samples without input tags.
        b_tags.push(tag(0, "generated"));
        (a + b, a_tags, a - b, b_tags)
    }
}

fn tag(pos: usize, key: &str) -> Tag {
    Tag::new(pos, key, TagValue::String(key.to_string()))
}

fn input(samples: &[u32], tags: &[Tag]) -> Result<ReadStream<u32>> {
    let (writer, reader) = new_stream()?;
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
    let window = sum.read_buf()?;
    let tags = window.tags().cloned().collect::<Vec<_>>();
    assert_eq!(&window.slice()[filler..], &[11, 22]);
    assert_eq!(tags, [tag(filler, "a"), tag(filler, "a2")]);
    window.consume_discard_tags(filler + 2);
    let window = difference.read_buf()?;
    let tags = window.tags().cloned().collect::<Vec<_>>();
    assert_eq!(window.slice(), &[9, 18]);
    let expected = if tagged {
        vec![tag(0, "generated"), tag(1, "b"), tag(1, "generated")]
    } else {
        vec![tag(0, "a"), tag(0, "a2")]
    };
    assert_eq!(tags, expected);
    window.consume_discard_tags(2);

    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    let window = sum.read_buf()?;
    let tags = window.tags().cloned().collect::<Vec<_>>();
    assert_eq!(window.slice(), &[33, 44, 55, 66]);
    assert_eq!(tags, [tag(0, "a3"), tag(3, "a4")]);
    window.consume_discard_tags(4);
    let window = difference.read_buf()?;
    let tags = window.tags().cloned().collect::<Vec<_>>();
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
    window.consume_discard_tags(4);
    Ok(())
}

#[test]
fn plain_sync_forwards_first_input_tags_across_batches() -> Result<()> {
    let (a, b) = inputs()?;
    let (mut block, sum, difference) = Plain::new(a, b)?;
    let filler = fill_output(&block.sum)?;
    check_batches(&mut block, sum, difference, filler, false)?;
    let remaining = block.b.read_buf()?;
    let tags = remaining.tags().cloned().collect::<Vec<_>>();
    assert_eq!(remaining.slice(), &[7]);
    assert_eq!(tags, [tag(0, "unconsumed")]);
    Ok(())
}

#[test]
fn tagged_sync_groups_each_input_and_preserves_output_tag_order() -> Result<()> {
    let (a, b) = inputs()?;
    let (mut block, sum, difference) = Tagged::new(a, b)?;
    let filler = fill_output(&block.sum)?;
    check_batches(&mut block, sum, difference, filler, true)
}

#[test]
fn tagged_sync_can_generate_tags_without_input_tags() -> Result<()> {
    let a = input(&[10, 20, 30], &[])?;
    let b = input(&[1, 2, 3], &[])?;
    let (mut block, sum, difference) = Tagged::new(a, b)?;
    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    let window = sum.read_buf()?;
    assert_eq!(window.slice(), &[11, 22, 33]);
    assert!(window.tags().len() == 0);
    let window = difference.read_buf()?;
    let tags = window.tags().cloned().collect::<Vec<_>>();
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
    let (mut block, sum, difference) = Plain::new(a, b)?;
    assert!(matches!(block.work()?, BlockRet::WaitForStream(_, 1)));
    assert!(sum.read_buf()?.tags().len() == 0);
    assert!(difference.read_buf()?.tags().len() == 0);
    Ok(())
}

#[derive(rustradio_macros::Block)]
#[rustradio(new, sync_tag)]
struct GeneratedTags {
    #[rustradio(in)]
    src: ReadStream<u32>,
    #[rustradio(out)]
    first: WriteStream<u32>,
    #[rustradio(out)]
    second: WriteStream<u32>,
}

impl GeneratedTags {
    fn process_sync_tags(&self, sample: u32, _: Vec<Tag>) -> (u32, Vec<Tag>, u32, Vec<Tag>) {
        (
            sample,
            vec![tag(0, &format!("first-{sample}"))],
            sample,
            vec![
                tag(0, &format!("second-{sample}")),
                tag(0, &format!("duplicate-{sample}")),
            ],
        )
    }
}

#[test]
fn owned_callback_tags_keep_positions_and_order_across_batches() -> Result<()> {
    for input_tags in [vec![], vec![tag(0, "input")]] {
        let source = input(&[10, 20, 30], &input_tags)?;
        let (mut block, first, second) = GeneratedTags::new(source)?;
        let filler = fill_output(&block.first)?;
        for (samples, padding) in [(&[10, 20][..], filler), (&[30][..], 0)] {
            block.work()?;
            let window = first.read_buf()?;
            let tags = window.tags().cloned().collect::<Vec<_>>();
            assert_eq!(&window.slice()[padding..], samples);
            assert_eq!(
                tags,
                samples
                    .iter()
                    .enumerate()
                    .map(|(pos, sample)| { tag(padding + pos, &format!("first-{sample}")) })
                    .collect::<Vec<_>>()
            );
            window.consume_discard_tags(padding + samples.len());
            let window = second.read_buf()?;
            let tags = window.tags().cloned().collect::<Vec<_>>();
            assert_eq!(window.slice(), samples);
            assert_eq!(
                tags,
                samples
                    .iter()
                    .enumerate()
                    .flat_map(|(pos, sample)| {
                        [
                            tag(pos, &format!("second-{sample}")),
                            tag(pos, &format!("duplicate-{sample}")),
                        ]
                    })
                    .collect::<Vec<_>>()
            );
            window.consume_discard_tags(samples.len());
        }
    }
    Ok(())
}

#[test]
fn map_appends_tag_and_moves_string_storage() -> Result<()> {
    let (writer, reader) = new_stream()?;
    let original = Tag::new(0, "channel", TagValue::String("packet".into()));
    let TagValue::String(value) = original.val() else {
        unreachable!()
    };
    let pointers = (original.key().as_ptr(), value.as_ptr());
    let mut window = writer.write_buf()?;
    window.fill_from_slice(&[42u32]);
    window.produce(1, vec![original]);
    let (mut block, output) = rustradio::convert::Map::new(reader, "mark", |sample, mut tags| {
        tags.push(tag(0, "marker"));
        (sample, tags)
    })?;
    block.work()?;
    let tags = output.read_buf()?.consume(1);
    assert_eq!(tags.len(), 2);
    let TagValue::String(value) = tags[0].val() else {
        unreachable!()
    };
    assert_eq!((tags[0].key().as_ptr(), value.as_ptr()), pointers);
    assert_eq!(tags[1], tag(0, "marker"));
    Ok(())
}

#[test]
fn callback_panic_consumes_only_samples_handed_to_it() -> Result<()> {
    let (writer, reader) = new_stream()?;
    let mut window = writer.write_buf()?;
    window.fill_from_slice(&[1u32, 2, 3]);
    window.produce(3, vec![tag(1, "panicking"), tag(2, "remaining")]);
    let (mut block, output) =
        rustradio::convert::Map::new(reader, "panic once", |sample, tags| {
            if sample == 2 {
                assert_eq!(tags, [tag(0, "panicking")]);
                panic!("callback panicked");
            }
            (sample, tags)
        })?;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        block.work().unwrap();
    }));
    assert_eq!(
        panic.unwrap_err().downcast_ref::<&str>(),
        Some(&"callback panicked")
    );
    assert!(output.read_buf()?.is_empty());
    block.work()?;
    let window = output.read_buf()?;
    assert_eq!(window.slice(), &[3]);
    assert_eq!(window.consume(1), [tag(0, "remaining")]);
    Ok(())
}
