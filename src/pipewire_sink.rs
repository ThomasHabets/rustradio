//! Pipewire sink.
use crate::block::{Block, BlockRet};
use crate::stream::ReadStream;
use crate::{Float, Result};

/// Pipewire sink builder.
///
/// Setting audio rate is mandatory.
#[derive(Default)]
#[must_use]
pub struct PipewireSinkBuilder {
    audio_rate: u32,
}

impl PipewireSinkBuilder {
    /// Build the `PipewireSink` block.
    pub fn build(self, src: ReadStream<Float>) -> Result<PipewireSink> {
        let p = PipewireSink::new(src, self.audio_rate)?;
        Ok(p)
    }
    /// Set desired audio rate.
    ///
    /// E.g. 44100.
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
    /// Create a builder.
    pub fn builder() -> PipewireSinkBuilder {
        PipewireSinkBuilder::default()
    }
    fn new(src: ReadStream<Float>, _audio_rate: u32) -> Result<Self> {
        Ok(Self { src })
    }
}

impl Block for PipewireSink {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        todo!()
    }
}
