//! Pipewire source.

use crate::block::{Block, BlockRet};
use crate::stream::WriteStream;
use crate::{Float, Result};

pub struct PipewireSourceBuilder {}

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct PipewireSource {
    #[rustradio(out)]
    dst: WriteStream<Float>,
}

impl Block for PipewireSource {
    fn work(&mut self) -> Result<BlockRet> {
        let _ = self.dst;
        todo!()
    }
}
