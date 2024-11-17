/*! Graphs contain blocks connected by streams, and run them.
 */
use std::time::Instant;

use anyhow::Result;
use log::{info, trace};

use crate::block::{Block, BlockRet};

/**
A graph is a thing that RustRadio runs, to let blocks "talk to each
other" via streams.

# Example

```
use rustradio::graph::Graph;
use rustradio::Complex;
use rustradio::blocks::{FileSource,RtlSdrDecode,AddConst,NullSink};
let src = Box::new(FileSource::<u8>::new("/dev/null", false)?);
let dec = Box::new(RtlSdrDecode::new(src.out()));
let add = Box::new(AddConst::new(dec.out(), Complex::new(1.1, 2.0)));
let sink = Box::new(NullSink::new(add.out()));
let mut g = Graph::new();
g.add(src);
g.add(dec);
g.add(add);
g.add(sink);
g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
pub struct Graph {
    blocks: Vec<Box<dyn Block>>,
    cancel_token: CancellationToken,
    times: Vec<std::time::Duration>,
}

impl Graph {
    /// Create a new flowgraph.
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            times: Vec::new(),
            cancel_token: CancellationToken::new(),
        }
    }

    /// Add a block to the flowgraph.
    pub fn add(&mut self, b: Box<dyn Block>) {
        self.blocks.push(b);
    }

    /// Run the graph until completion.
    pub fn run(&mut self) -> Result<()> {
        let st = Instant::now();
        self.times
            .resize(self.blocks.len(), std::time::Duration::default());
        let mut eof = vec![false; self.blocks.len()];
        loop {
            let mut done = true;
            let mut all_idle = true;
            if self.cancel_token.is_canceled() {
                break;
            }
            for (n, b) in self.blocks.iter_mut().enumerate() {
                if eof[n] {
                    continue;
                }
                let st = Instant::now();
                let ret = b.work()?;
                self.times[n] += st.elapsed();
                match ret {
                    BlockRet::Ok => {
                        // Block did something.
                        trace!("… {} was not starved", b.block_name());
                        done = false;
                        all_idle = false;
                    }
                    BlockRet::Pending => {
                        done = false;
                    }
                    BlockRet::Noop => {}
                    BlockRet::EOF => {
                        eof[n] = true;
                    }
                    BlockRet::InternalAwaiting => {
                        panic!("blocks must never return InternalAwaiting")
                    }
                };
            }
            if done {
                break;
            }
            if all_idle {
                let idle_sleep = std::time::Duration::from_millis(10);
                trace!("No output or consumption from any block. Sleeping a bit.");
                std::thread::sleep(idle_sleep);
            }
        }
        for line in self.generate_stats(st.elapsed()).split('\n') {
            if !line.is_empty() {
                info!("{}", line);
            }
        }
        Ok(())
    }

    /// Return a string with stats about where time went.
    pub fn generate_stats(&self, elapsed: std::time::Duration) -> String {
        let total = self
            .times
            .iter()
            .cloned()
            .sum::<std::time::Duration>()
            .as_secs_f64();
        let ml = self
            .blocks
            .iter()
            .map(|b| b.block_name().len())
            .max()
            .unwrap(); // unwrap: can only fail if block list is empty.
        let ml = std::cmp::max(ml, "Elapsed seconds".len());
        let elapsed = elapsed.as_secs_f64();

        let dashes = "-".repeat(ml + 20) + "\n";
        let (secw, secd) = (10, 3);
        let (pw, pd) = (7, 2);

        let mut s: String = format!("{:<width$}    Seconds  Percent\n", "Block name", width = ml);
        s.push_str(&dashes);
        for (n, b) in self.blocks.iter().enumerate() {
            s.push_str(&format!(
                "{:<width$} {:secw$.secd$} {:>pw$.pd$}%\n",
                b.block_name(),
                self.times[n].as_secs_f32(),
                100.0 * self.times[n].as_secs_f64() / total,
                width = ml,
            ));
        }
        s.push_str(&dashes);
        s.push_str(&format!(
            "{:<width$} {total:secw$.secd$} {:>pw$.pd$}%\n",
            "All blocks",
            100.0 * total / elapsed,
            width = ml,
        ));
        s.push_str(&format!(
            "{:<width$} {:secw$.secd$} {:>pw$.pd$}%\n",
            "Non-block time",
            elapsed - total,
            100.0 * (elapsed - total) / elapsed,
            width = ml,
        ));
        s.push_str(&format!(
            "{:<width$} {elapsed:secw$.secd$} {:>pw$.pd$}%\n",
            "Elapsed seconds",
            100.0,
            width = ml,
        ));
        s
    }

    /// Return a cancellation token, for asynchronously stopping the
    /// graph, for example if the user presses Ctrl-C.
    ///
    ///
    /// # Example
    ///
    /// ```no_run
    /// let mut g = rustradio::graph::Graph::new();
    /// let cancel = g.cancel_token();
    /// ctrlc::set_handler(move || {
    ///     cancel.cancel();
    /// }).expect("failed to set Ctrl-C handler");
    /// g.run()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

/** A handle to be able to stop the Graph. For example when the user
presses Ctrl-C.

```
use rustradio::graph::CancellationToken;
use std::thread;

// Token normally extracted from graph.cancel_token().
let token = CancellationToken::new();

// Confirm it defaults to not cancelled.
assert!(!token.is_canceled());

// Start a thread that will cancel the token.
let tt = token.clone();
assert!(!token.is_canceled());
assert!(!tt.is_canceled());
thread::spawn(move || {
   tt.cancel();
});

// This would normally be graph.run();
while !token.is_canceled() {}
```
*/
#[derive(Clone)]
pub struct CancellationToken {
    inner: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    /// Create new cancellation token.
    pub fn new() -> Self {
        CancellationToken {
            inner: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Mark the token cancelled.
    pub fn cancel(&self) {
        self.inner.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if the token is cancelled.
    pub fn is_canceled(&self) -> bool {
        self.inner.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}
