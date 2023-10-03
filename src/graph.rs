/*! Graphs contain blocks connected by streams, and run them.
 */
use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType};

/// When adding a block to a graph, this handle is handed back to be
/// used for connecting blocks together.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct BlockHandle(usize);

#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
struct StreamHandle(usize);

type Port = usize;

/**
A graph is a thing that RustRadio runs, to let blocks "talk to each
other" via streams.

# Example

```
use rustradio::graph::Graph;
use rustradio::Complex;
use rustradio::stream::StreamType;
use rustradio::blocks::{FileSource,RtlSdrDecode,AddConst,NullSink};
# fn main() -> anyhow::Result<()> {
let mut g = Graph::new();
let src = g.add(Box::new(FileSource::<u8>::new("/dev/null", false)?));
let dec = g.add(Box::new(RtlSdrDecode::new()));
let add = g.add(Box::new(AddConst::new(Complex::new(1.1, 2.0))));
let sink = g.add(Box::new(NullSink::<Complex>::new()));
g.connect(StreamType::new_u8(), src, 0, dec, 0);
g.connect(StreamType::new_complex(), dec, 0, add, 0);
g.connect(StreamType::new_complex(), add, 0, sink, 0);
g.run()?;
# Ok(())
# }
```
*/
pub struct Graph {
    blocks: Vec<Box<dyn Block>>,
    streams: Vec<StreamType>,

    outputs: HashMap<BlockHandle, Vec<(Port, StreamHandle)>>,
    inputs: HashMap<BlockHandle, Vec<(Port, StreamHandle)>>,
}

impl Graph {
    /// Create new empty graph.
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            outputs: HashMap::new(),
            inputs: HashMap::new(),
            streams: Vec::new(),
        }
    }
    /// Add a block to the graph, returning a handle to it.
    pub fn add(&mut self, b: Box<dyn Block>) -> BlockHandle {
        self.blocks.push(b);
        BlockHandle(self.blocks.len() - 1)
    }
    /// Connect two blocks (by handle).
    ///
    /// Output port p1 on block b1 becomes connected to input port p2
    /// on block b2.
    ///
    /// The stream needs to be provided, such as `StreamType::new_complex()`.
    pub fn connect(
        &mut self,
        stream: StreamType,
        b1: BlockHandle,
        p1: Port,
        b2: BlockHandle,
        p2: Port,
    ) {
        let s = self.streams.len();
        self.streams.push(stream);
        self.outputs
            .entry(b1)
            .or_default()
            .push((p1, StreamHandle(s)));
        self.inputs
            .entry(b2)
            .or_default()
            .push((p2, StreamHandle(s)));
    }

    /// Run the graph, until there's no more data to process.
    pub fn run(&mut self) -> Result<()> {
        for input in self.inputs.values_mut() {
            input.sort();
        }
        for output in &mut self.outputs.values_mut() {
            output.sort();
        }
        let mut iss = Vec::new();
        let mut oss = Vec::new();
        for (n, _) in self.blocks.iter().enumerate() {
            let mut is = InputStreams::new();
            let mut os = OutputStreams::new();
            if let Some(es) = self.inputs.get(&BlockHandle(n)) {
                let mut expected = 0;
                for (n, e) in es {
                    while expected != *n {
                        is.add_stream(StreamType::new_disconnected());
                        expected += 1;
                    }
                    is.add_stream(self.streams[e.0].clone());
                    expected = *n + 1;
                }
            }
            if let Some(es) = self.outputs.get(&BlockHandle(n)) {
                let mut expected = 0;
                for (n, e) in es {
                    while expected != *n {
                        os.add_stream(StreamType::new_disconnected());
                        expected += 1;
                    }
                    os.add_stream(self.streams[e.0].clone());
                    expected = *n + 1;
                }
            }
            iss.push(is);
            oss.push(os);
        }
        while !self.run_one(&mut iss, &mut oss)? {}
        Ok(())
    }

    fn run_one(&mut self, iss: &mut [InputStreams], oss: &mut [OutputStreams]) -> Result<bool> {
        let mut done = true;
        let st_loop = Instant::now();
        let mut processed = 0;
        for (n, b) in self.blocks.iter_mut().enumerate() {
            let st = Instant::now();
            let os = &mut oss[n];
            if !os.is_empty() && os.all_outputs_full() {
                debug!(
                    "work() skipped for {} because all outputs are full",
                    b.block_name()
                );
                continue;
            }
            let is = &mut iss[n];

            let insamples = is.sum_available();
            let before_outsamples = os.sum_available();

            let eof = matches!(b.work(is, os)?, BlockRet::EOF);
            processed += insamples - is.sum_available();
            processed += os.sum_available() - before_outsamples;
            let outsamples = os.sum_available();
            debug!(
                "work() done for {}, processing {} -> {}. Took {:?}",
                b.block_name(),
                insamples,
                outsamples,
                st.elapsed()
            );

            // If source block then only done if EOF.
            if is.is_empty() && !eof {
                done = false;
            }
            for n in 0..os.len() {
                if os.get(n).available() > 0 {
                    done = false;
                }
            }
        }
        debug!(
            "Graph loop end. done status: {done}. Processed in/out: {} Took {:?}",
            processed,
            st_loop.elapsed()
        );
        if processed == 0 {
            let ten_millis = std::time::Duration::from_millis(10);
            debug!("No output or consumption from any block. Sleeping a bit.");
            std::thread::sleep(ten_millis);
        }
        Ok(done)
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}
