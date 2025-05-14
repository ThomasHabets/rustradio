use log::{debug, error, info};

use crate::Result;
use crate::block::{Block, BlockRet};
use crate::graph::{CancellationToken, GraphRunner};

/// Async Graph executor.
///
/// # Example
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), anyhow::Error> {
/// eprintln!("Hello");
/// use rustradio::graph::GraphRunner;
/// use rustradio::agraph::AsyncGraph;
/// use rustradio::blocks::{VectorSource,NullSink};
/// let (src, prev) = VectorSource::new(vec![0u8; 10]);
/// let sink = NullSink::new(prev);
/// let mut g = AsyncGraph::new();
/// g.add(Box::new(src));
/// g.add(Box::new(sink));
/// g.run_async().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Default)]
pub struct AsyncGraph {
    blocks: Vec<Box<dyn Block>>,
    cancel_token: CancellationToken,
}

impl AsyncGraph {
    /// Create a new async flowgraph.
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn run_async(&mut self) -> Result<()> {
        let mut tasks = Vec::new();
        while let Some(mut b) = self.blocks.pop() {
            let cancel_token = self.cancel_token.clone();
            tasks.push(tokio::spawn(async move {
                while !cancel_token.is_canceled() {
                    let name = b.block_name().to_string();
                    let ret = match b.work() {
                        Ok(v) => v,
                        Err(e) => {
                            error!("Block work function failed: {e}");
                            return Err(e);
                        }
                    };
                    match ret {
                        BlockRet::Again => {}
                        BlockRet::EOF => break,
                        BlockRet::WaitForStream(stream, need) => {
                            debug!("{name} wait for stream");
                            let eof = stream.wait_async(need).await;
                            drop(ret);
                            //if b.eof() || eof {
                            if eof {
                                break;
                            }
                        }
                        BlockRet::WaitForFunc(_) => {}
                        BlockRet::Pending => {}
                    }
                }
                info!("Block {} done", b.block_name());
                Ok(())
            }));
        }
        for task in tasks.into_iter() {
            match task.await {
                Ok(res) => info!("Task exited with status {res:?}"),
                Err(e) => error!("Task failed: {e}!"),
            }
        }
        Ok(())
    }
}

impl GraphRunner for AsyncGraph {
    fn add(&mut self, b: Box<dyn Block + Send>) {
        self.blocks.push(b);
    }

    fn run(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn generate_stats(&self) -> Option<String> {
        None
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nullsink() -> Result<()> {
        eprintln!("Hello");
        use crate::agraph::AsyncGraph;
        use crate::blocks::{NullSink, VectorSource};
        use crate::graph::GraphRunner;
        let (src, prev) = VectorSource::new(vec![0u8; 10]);
        let sink = NullSink::new(prev);
        let mut g = AsyncGraph::new();
        g.add(Box::new(src));
        g.add(Box::new(sink));
        g.run_async().await?;
        Ok(())
    }

    #[tokio::test]
    async fn double() -> Result<()> {
        use crate::agraph::AsyncGraph;
        use crate::blocks::{Map, VectorSink, VectorSource};
        use crate::graph::GraphRunner;
        let (src, prev) = VectorSource::new(vec![1u8, 2, 3]);
        let (mul, prev) = Map::new(prev, "double", move |x| x * 2);
        let sink = VectorSink::new(prev, 100);
        let hook = sink.hook();
        let mut g = AsyncGraph::new();
        g.add(Box::new(src));
        g.add(Box::new(mul));
        g.add(Box::new(sink));
        g.run_async().await?;
        assert_eq!(hook.data().samples(), [2, 4, 6]);
        Ok(())
    }
}
