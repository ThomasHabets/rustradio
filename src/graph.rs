use std::collections::HashMap;

use anyhow::Result;
use log::debug;

use crate::block::{Block, BlockRet};
use crate::stream::{InputStreams, OutputStreams, StreamType};

type BlockHandle = usize;
type StreamHandle = usize;
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
        self.blocks.len() - 1
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
        self.outputs.entry(b1).or_default().push((p1, s));
        self.inputs.entry(b2).or_default().push((p2, s));
        // TODO: sort them.
    }

    pub fn run(&mut self) -> Result<()> {
        while !self.run_one()? {}
        Ok(())
    }

    fn run_one(&mut self) -> Result<bool> {
        let mut done = true;
        for (n, b) in self.blocks.iter_mut().enumerate() {
            let mut is = InputStreams::new();
            let mut os = OutputStreams::new();
            if let Some(es) = self.inputs.get(&n) {
                // TODO: support port gaps.
                for (_, e) in es {
                    is.add_stream(self.streams[*e].clone());
                }
            }
            if let Some(es) = self.outputs.get(&n) {
                // TODO: support port gaps.
                for (_, e) in es {
                    os.add_stream(self.streams[*e].clone());
                }
            }
            let eof = matches!(b.work(&mut is, &mut os)?, BlockRet::EOF);

            // If source block then only done if EOF.
            if is.is_empty() && !eof {
                done = false;
            }
            for n in 0..os.len() {
                if os.get(n).available() > 0 {
                    done = false;
                }
            }
            debug!("work() done for {}", b.block_name());
        }
        debug!("done status: {done}");
        Ok(done)
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}
