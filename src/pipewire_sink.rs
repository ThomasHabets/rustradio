//! Pipewire sink.
use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;
use crate::{Float, Result};

/// Pipewire sink builder.
///
/// Setting audio rate is mandatory.
#[derive(Default)]
pub struct PipewireSinkBuilder {
    audio_rate: u32,
}

impl PipewireSinkBuilder {
    pub fn build(self, src: ReadStream<Float>) -> Result<PipewireSink> {
        let p = PipewireSink::new(src, self.audio_rate)?;
        Ok(p)
    }
    pub fn audio_rate(mut self, r: u32) -> Self {
        self.audio_rate = r;
        self
    }
}

/// Pipewire sink.
///
/// TODO: draw the rest of the owl.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct PipewireSink {
    #[rustradio(in)]
    src: ReadStream<Float>,
}

impl PipewireSink {
    pub fn builder() -> PipewireSinkBuilder {
        PipewireSinkBuilder::default()
    }
    fn new(src: ReadStream<Float>, _audio_rate: u32) -> Result<Self> {
        Ok(Self { src })
    }
}

impl Block for PipewireSink {
    fn work(&mut self) -> Result<BlockRet> {
        todo!()
    }
}
