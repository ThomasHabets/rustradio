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
            // Pending requests an external wake, but another block may still
            // have local work. Again or dropping a block requires another pass
            // before waiting, so queued samples and stream closure can propagate.
            let mut progress = false;
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
                    BlockRet::Again => {
                        done = false;
                        progress = true;
                    }
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
                    progress = true;
                }
            }
            if done {
                info!("Wasm graph: All done");
                return Ok(());
            }
            if need_more && !progress {
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
    use crate::stream::{ReadStream, WriteStream};
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
    struct ExternalSource {
        #[rustradio(out)]
        dst: WriteStream<f32>,
        produced: bool,
    }
    impl Block for ExternalSource {
        fn work(&mut self) -> crate::Result<BlockRet<'_>> {
            if self.produced {
                return Ok(BlockRet::Pending);
            }
            let mut output = self.dst.write_buf()?;
            output.slice()[0] = 1.0;
            output.produce(1, &[]);
            self.produced = true;
            Ok(BlockRet::Again)
        }
    }
    #[derive(rustradio_macros::Block)]
    #[rustradio(crate)]
    struct Steps {
        #[rustradio(in)]
        src: ReadStream<f32>,
        #[rustradio(out)]
        dst: WriteStream<f32>,
        steps: usize,
    }
    impl Block for Steps {
        fn work(&mut self) -> crate::Result<BlockRet<'_>> {
            let (input, _) = self.src.read_buf()?;
            if input.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            self.steps += 1;
            if self.steps < 4 {
                return Ok(BlockRet::Again);
            }
            let mut output = self.dst.write_buf()?;
            output.slice()[0] = input.slice()[0];
            output.produce(1, &[]);
            input.consume(1);
            Ok(BlockRet::Again)
        }
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
    fn external_pending_does_not_interrupt_local_progress() -> crate::Result<()> {
        let (writer, input) = crate::stream::new_stream();
        let (writer2, output) = crate::stream::new_stream();
        let sink = crate::vector_sink::VectorSink::new(output, 1);
        let result = sink.hook();
        let mut graph = WasmGraph::new();
        graph.add(Box::new(ExternalSource {
            dst: writer,
            produced: false,
        }));
        graph.add(Box::new(Steps {
            src: input,
            dst: writer2,
            steps: 0,
        }));
        graph.add(Box::new(sink));
        let (_poke, wake) = async_channel::bounded(1);
        let mut run = Box::pin(graph.run_async(wake));
        let waker = Waker::from(Arc::new(Noop));
        let mut cx = Context::from_waker(&waker);
        assert!(matches!(
            std::future::Future::poll(run.as_mut(), &mut cx),
            Poll::Pending
        ));
        assert_eq!(result.data().samples(), &[1.0]);
        Ok(())
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
