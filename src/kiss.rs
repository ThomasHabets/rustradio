use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, ReadStream, WriteStream};

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct KissDecode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<u8>>,
}

impl Block for KissDecode {
    fn work(&mut self) -> Result<BlockRet> {
        let _ = &self.src;
        let _ = &self.dst;
        todo!()
    }
}

#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct KissEncode {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
}

impl Block for KissEncode {
    fn work(&mut self) -> Result<BlockRet> {
        let _ = &self.src;
        let _ = &self.dst;
        todo!()
    }
}
