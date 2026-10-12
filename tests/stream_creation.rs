//! Check that buffer errors reach callers of manual and generated constructors.
use rustradio::block::{Block, BlockRet};
use rustradio::stream::{NCReadStream, NCWriteStream, ReadStream, WriteStream, new_stream};
use rustradio::{Result, Sample};

#[derive(rustradio_macros::Block)]
#[rustradio(new, bound = "T: Copy + Default + Send + Sync")]
struct Mixed<T> {
    #[rustradio(out)]
    samples: WriteStream<T>,
    #[rustradio(out)]
    messages: NCWriteStream<String>,
}
impl<T: Copy + Default + Send + Sync> Block for Mixed<T> {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let mut window = self.samples.write_buf()?;
        window.slice()[0] = T::default();
        window.produce(1, &[]);
        self.messages.push("mixed".to_string(), &[]);
        Ok(BlockRet::EOF)
    }
}

#[derive(rustradio_macros::Block)]
#[rustradio(new)]
struct Messages {
    #[rustradio(out)]
    messages: NCWriteStream<String>,
}
impl Block for Messages {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        self.messages.push("nocopy".to_string(), &[]);
        Ok(BlockRet::EOF)
    }
}

#[test]
fn unsupported_layout_returns_an_error_through_every_constructor() {
    // The fixed buffer byte size cannot hold a whole number of three-byte samples.
    assert!(new_stream::<[u8; 3]>().is_err());
    assert!(WriteStream::<[u8; 3]>::new().is_err());
    assert!(Mixed::<[u8; 3]>::new().is_err());
    assert!(rustradio::blocks::VectorSource::new(vec![ThreeBytes::default()]).is_err());
    // Configuring a builder must not allocate an output or panic on this layout.
    let builder = rustradio::blocks::VectorSource::builder(vec![ThreeBytes::default()]);
    assert!(builder.build().is_err());
}

#[test]
fn mixed_outputs_are_fallible_and_nocopy_outputs_remain_infallible() -> Result<()> {
    let (mut block, samples, messages): (Mixed<u8>, ReadStream<u8>, NCReadStream<String>) =
        Mixed::new()?;
    block.work()?;
    assert_eq!(samples.read_buf()?.0.slice(), &[0]);
    assert_eq!(messages.pop().unwrap().0, "mixed");
    let (mut block, messages): (Messages, NCReadStream<String>) = Messages::new();
    block.work()?;
    assert_eq!(messages.pop().unwrap().0, "nocopy");
    Ok(())
}

#[derive(Debug, Default, Clone, Copy)]
struct ThreeBytes([u8; 3]);
impl Sample for ThreeBytes {
    fn size() -> usize {
        3
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes[..3].try_into().unwrap()))
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.0);
    }
}
