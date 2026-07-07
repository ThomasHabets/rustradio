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
    blocks: Vec<Box<dyn Block>>,
}

impl WasmGraph {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn run_async(&mut self, rx: async_channel::Receiver<()>) -> crate::Result<()> {
        let mut eof = vec![false; self.blocks.len()];
        let rx = Box::pin(rx);
        loop {
            let mut done = true;
            let mut need_more = false;
            for (n, b) in self.blocks.iter_mut().enumerate() {
                let name = b.block_name().to_owned();
                trace!("Running graph node {name}");
                if eof[n] {
                    continue;
                }
                let ret = b.work()?;
                trace!("graph node {name} work ended");
                match ret {
                    BlockRet::EOF => {
                        eof[n] = true;
                        info!("Block({name}): EOF");
                    }
                    BlockRet::Again => done = false,
                    // TODO: Skip calling next time if conditions not met?
                    BlockRet::WaitForStream(s, _) => {
                        let closed = s.closed();
                        if b.eof() && closed {
                            eof[n] = true;
                        }
                    }
                    BlockRet::Pending => {
                        //info!("Block {name} returned Pending");
                        need_more = true;
                        done = false;
                    }
                }
            }
            if done {
                info!("Wasm graph: All done");
                return Ok(());
            }
            if need_more {
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
        self.blocks.push(b);
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
