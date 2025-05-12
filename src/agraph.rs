use crate::Result;
use crate::block::{Block, BlockRet};

pub async fn run(mut b: Box<dyn Block>) -> Result<()> {
    loop {
        let ret = b.work()?;
        match ret {
            BlockRet::Again => {}
            BlockRet::Pending => {}
            BlockRet::WaitForFunc(_) => {}
            BlockRet::WaitForStream(stream, need) => {
                stream.wait_async(need).await;
            }
            BlockRet::EOF => break,
        }
    }
    Ok(())
}
