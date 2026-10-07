//! Graph executor for WASM.
//!
//! Ideally this should be merged with the general `AsyncGraph`, but it can't have
//! any dependency on tokio, then.
//!
//! Also it should probably work more like `AsyncGraph` by spawning one async
//! task per block. We don't do that here because lack of sleep would busy loop
//! a lot. So in other words `AsyncGraph` should have spawning, timing, and
//! sleeping pluggable.
use log::{info, trace};

use crate::block::{Block, BlockRet};
use crate::graph::{CancellationToken, GraphRunner};

/// Graph executor for use in WASM.
///
/// It needs to be a bit special because it needs to be async, and not use
/// system stuff like clock.
///
/// Means we can't get much statistics.
///
/// Possibly this could be merged with the rustradio `AsyncGraph`.
#[derive(Default)]
pub struct WasmGraph {
    blocks: Vec<Option<Box<dyn Block>>>,
    // Names are captured when blocks are added, outside the work loop.
    block_names: Vec<String>,
}

impl WasmGraph {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn run_async(&mut self, rx: async_channel::Receiver<()>) -> crate::Result<()> {
        let rx = Box::pin(rx);
        loop {
            let mut done = true;
            let mut need_more = false;
            let mut dropped_block = false;
            for (n, slot) in self.blocks.iter_mut().enumerate() {
                let Some(b) = slot.as_mut() else { continue };
                let mut finished = false;
                let name = &self.block_names[n];
                trace!("Running graph node {name}");
                let ret = b.work()?;
                trace!("graph node {name} work ended");
                match ret {
                    BlockRet::EOF => {
                        finished = true;
                        info!("Block({name}): EOF");
                    }
                    BlockRet::Again => done = false,
                    // TODO: Skip calling next time if conditions not met?
                    BlockRet::WaitForStream(s, _) => {
                        let closed = s.closed();
                        if b.eof() && closed {
                            finished = true;
                        }
                    }
                    BlockRet::Pending => {
                        //info!("Block {name} returned Pending");
                        need_more = true;
                        done = false;
                    }
                }
                if finished {
                    slot.take();
                    done = false;
                    dropped_block = true;
                }
            }
            if done {
                info!("Wasm graph: All done");
                return Ok(());
            }
            if need_more && !dropped_block {
                trace!("Graph: About to wait for more somethings");
                if let Err(e) = rx.recv().await {
                    info!("Graph: recv error: {e:?}");
                    // This can only happen if the sender crashed. If the worker
                    // crashed, then there's no point in continuing the graph
                    // connected to nothing.
                    return Err(crate::Error::msg("recv()"));
                }
                trace!("Graph: Got woken up");
            }
        }
    }
}

impl GraphRunner for WasmGraph {
    fn add(&mut self, b: Box<dyn Block + Send>) {
        self.block_names.push(b.block_name().to_owned());
        self.blocks.push(Some(b));
    }
    fn run(&mut self) -> crate::Result<()> {
        todo!()
    }
    fn generate_stats(&self) -> Option<String> {
        None
    }
    fn cancel_token(&self) -> CancellationToken {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::ReadStream;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }

    #[derive(rustradio_macros::Block)]
    #[rustradio(crate)]
    struct ClosingSink {
        #[rustradio(in)]
        src: ReadStream<f32>,
        finished: Arc<AtomicBool>,
    }
    impl Block for ClosingSink {
        fn work(&mut self) -> crate::Result<BlockRet<'_>> {
            if self.src.eof() {
                self.finished.store(true, Ordering::SeqCst);
                return Ok(BlockRet::EOF);
            }
            let (input, _) = self.src.read_buf()?;
            if input.is_empty() {
                return Ok(BlockRet::Pending);
            }
            let n = input.len();
            input.consume(n);
            Ok(BlockRet::Again)
        }
    }
    #[test]
    fn completed_source_closes_output_before_graph_finishes() -> crate::Result<()> {
        let (source, input) = crate::vector_source::VectorSource::new(vec![1.0f32]);
        let finished = Arc::new(AtomicBool::new(false));
        let mut graph = WasmGraph::new();
        graph.add(Box::new(ClosingSink {
            src: input,
            finished: finished.clone(),
        }));
        graph.add(Box::new(source));
        let (_poke, wake) = async_channel::bounded(1);
        let mut run = Box::pin(graph.run_async(wake));
        let waker = Waker::from(Arc::new(Noop));
        let mut cx = Context::from_waker(&waker);
        assert!(matches!(
            std::future::Future::poll(run.as_mut(), &mut cx),
            Poll::Ready(Ok(()))
        ));
        assert!(finished.load(Ordering::SeqCst));
        Ok(())
    }
}
