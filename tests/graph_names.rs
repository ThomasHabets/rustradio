//! Names are captured once, even across multiple scheduler passes and reports.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustradio::Result;
use rustradio::block::{Block, BlockEOF, BlockRet};
use rustradio::graph::GraphRunner;

#[derive(rustradio_macros::Block)]
#[rustradio(custom_name, noeof)]
struct NamedBlock {
    name_calls: Arc<AtomicUsize>,
    work_calls: usize,
}

impl NamedBlock {
    fn custom_name(&self) -> &str {
        self.name_calls.fetch_add(1, Ordering::Relaxed);
        "CachedBlock"
    }
}

impl BlockEOF for NamedBlock {
    fn eof(&mut self) -> bool {
        false
    }
}

impl Block for NamedBlock {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        self.work_calls += 1;
        Ok(if self.work_calls < 3 {
            BlockRet::Again
        } else {
            BlockRet::EOF
        })
    }
}

#[cfg(not(feature = "wasm"))]
#[test]
fn graph_reuses_names_for_work_and_stats() -> Result<()> {
    use rustradio::graph::Graph;

    let calls = Arc::new(AtomicUsize::new(0));
    let mut graph = Graph::new();
    graph.add(Box::new(NamedBlock {
        name_calls: calls.clone(),
        work_calls: 0,
    }));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    graph.run()?;
    assert!(graph.generate_stats().unwrap().contains("CachedBlock"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    Ok(())
}

#[cfg(feature = "wasm")]
#[test]
fn wasm_graph_reuses_names_across_work_calls() -> Result<()> {
    use rustradio::wasm::wasm_graph::WasmGraph;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    let calls = Arc::new(AtomicUsize::new(0));
    let mut graph = WasmGraph::new();
    graph.add(Box::new(NamedBlock {
        name_calls: calls.clone(),
        work_calls: 0,
    }));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let (_sender, receiver) = async_channel::bounded(1);
    let mut run = std::pin::pin!(graph.run_async(receiver));
    // This graph never waits for external input, so one poll completes it.
    let Poll::Ready(result) = run.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("graph unexpectedly waited for input");
    };
    result?;
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    Ok(())
}
