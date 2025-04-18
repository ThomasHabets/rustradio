use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream};
use crate::{Complex, Result};

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Fft {
    #[rustradio(in)]
    src: NCReadStream<Vec<Complex>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<Complex>>,
}

impl Block for Fft {
    fn work(&mut self) -> Result<BlockRet> {
        _ = self.src;
        _ = self.dst;
        todo!()
    }
}
