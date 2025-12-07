use log::{debug, error, info};

use crate::Result;
use crate::block::{Block, BlockRet};
use crate::graph::{CancellationToken, GraphRunner};

const SLEEP_TIME: tokio::time::Duration = tokio::time::Duration::from_millis(10);

#[cfg(feature = "tokio-unstable")]
pub fn spawn<F>(name: &str, future: F) -> Result<tokio::task::JoinHandle<F::Output>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    Ok(tokio::task::Builder::new().name(name).spawn(future)?)
}

#[cfg(not(feature = "tokio-unstable"))]
pub fn spawn<F>(_name: &str, future: F) -> Result<tokio::task::JoinHandle<F::Output>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    Ok(tokio::spawn(future))
}

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
    /// Run the graph until completion.
    pub async fn run_async(&mut self) -> Result<()> {
        let mut tasks = Vec::new();
        while let Some(mut b) = self.blocks.pop() {
            let cancel_token = self.cancel_token.clone();
            let name = b.block_name().to_string();
            tasks.push(spawn(&name, async move {
                let name = b.block_name().to_string();
                while !cancel_token.is_canceled() {
                    //log::trace!("Still running: {name}");
                    let ret = match b.work() {
                        Ok(v) => v,
                        Err(e) => {
                            error!("Block work function failed: {e}");
                            return Err(e);
                        }
                    };
                    match ret {
                        BlockRet::Again => {
                            debug!("{name} Again");
                        }
                        BlockRet::EOF => break,
                        BlockRet::WaitForStream(stream, need) => {
                            //debug!("{name} wait for stream");
                            drop(ret);
                            let eof = stream.wait_async(need).await;
                            if b.eof() || eof {
                                break;
                            }
                        }
                        BlockRet::WaitForFunc(_) => {
                            //debug!("{name} WaitForFunc");
                            drop(ret);
                            if b.eof() {
                                break;
                            }
                        }
                        BlockRet::Pending => {
                            drop(ret);
                            tokio::time::sleep(SLEEP_TIME).await;
                            //debug!("{name} Pending");
                        }
                    }
                }
                info!("Block {name} done");
                drop(b);
                Ok(name)
            })?);
        }
        for task in tasks.into_iter() {
            match task.await {
                Ok(name) => info!("Task exited with status {name:?}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;
    use std::borrow::Cow;

    #[tokio::test]
    async fn nullsink() -> Result<()> {
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
        let (mul, prev) = Map::new(prev, "double", move |x, tags| (x * 2, Cow::Borrowed(tags)));
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

    #[tokio::test]
    async fn big() -> Result<()> {
        use crate::agraph::AsyncGraph;
        use crate::blocks::{Map, VectorSink, VectorSource};
        use crate::graph::GraphRunner;
        let n = 1_000_000;
        let (src, prev) = VectorSource::new(vec![Complex::new(1.0, 0.0); n]);
        let (mul, prev) = Map::new(prev, "double", move |x, tags| {
            (x * 2.0, Cow::Borrowed(tags))
        });
        let sink = VectorSink::new(prev, n * 2);
        let hook = sink.hook();
        let mut g = AsyncGraph::new();
        g.add(Box::new(src));
        g.add(Box::new(mul));
        g.add(Box::new(sink));
        g.run_async().await?;
        assert_eq!(hook.data().samples(), vec![Complex::new(2.0, 0.0); n]);
        Ok(())
    }
}
