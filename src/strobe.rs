//! Message strobe.
use crate::Result;
use crate::block::{Block, BlockRet};
use crate::stream::NCWriteStream;

/// Message strobe.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Strobe<T: Send + Clone> {
    #[rustradio(out)]
    dst: NCWriteStream<T>,
    #[rustradio(default)]
    last: Option<std::time::Instant>,
    period: std::time::Duration,
    data: T,
}

impl<T: Send + Clone> Block for Strobe<T> {
    fn work(&mut self) -> Result<BlockRet> {
        let now = std::time::Instant::now();
        match self.last {
            None => {}
            Some(last) if now < last + self.period => return Ok(BlockRet::Pending),
            Some(_) => {}
        }
        // TODO: Add tags?
        self.dst.push(self.data.clone(), &[]);
        self.last = Some(now);
        Ok(BlockRet::Pending)
    }
}
