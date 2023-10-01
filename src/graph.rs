use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct BlockHandle(usize);

#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
struct StreamHandle(usize);

type Port = usize;

pub struct Graph {
    blocks: Vec<Box<dyn Block>>,
    streams: Vec<StreamType>,

    outputs: HashMap<BlockHandle, Vec<(Port, StreamHandle)>>,
    inputs: HashMap<BlockHandle, Vec<(Port, StreamHandle)>>,
}

impl Graph {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            outputs: HashMap::new(),
            inputs: HashMap::new(),
            streams: Vec::new(),
        }
    }
    pub fn add(&mut self, b: Box<dyn Block>) -> BlockHandle {
        self.blocks.push(b);
        BlockHandle(self.blocks.len() - 1)
    }
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
            "Graph loop end. done status: {done}. Took {:?}",
            st_loop.elapsed()
        );
        if processed == 0 {
            let ten_millis = std::time::Duration::from_millis(10);
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