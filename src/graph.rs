/*! Graphs contain blocks connected by streams, and run them.
 */
use std::time::Instant;

use crate::{Error, Result};
use log::{info, trace};

use crate::block::{Block, BlockRet};

/**
Abstraction over graph executors.
*/
pub trait GraphRunner {
    /// Add a block to the graph.
    fn add(&mut self, b: Box<dyn Block + Send>);

    /// Run the graph.
    ///
    /// Runs the graph until all the blocks are "done", or until the graph is
    /// cancelled.
    fn run(&mut self) -> Result<()>;

    /// Return a string with stats about where time went.
    fn generate_stats(&self) -> Option<String>;

    /// Return a cancellation token, for asynchronously stopping the
    /// graph, for example if the user presses Ctrl-C.
    ///
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rustradio::graph::GraphRunner;
    /// let mut g = rustradio::graph::Graph::new();
    /// let cancel = g.cancel_token();
    /// ctrlc::set_handler(move || {
    ///     cancel.cancel();
    /// }).expect("failed to set Ctrl-C handler");
    /// g.run()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    fn cancel_token(&self) -> CancellationToken;
}

/**
A graph is a thing that RustRadio runs, to let blocks "talk to each
other" via streams.

# Example

```
use rustradio::graph::{Graph, GraphRunner};
use rustradio::Complex;
use rustradio::blocks::{FileSource,RtlSdrDecode,AddConst,NullSink};
let (src, src_out) = FileSource::new("/dev/null")?;
let (dec, dec_out) = RtlSdrDecode::new(src_out);
let (add, add_out) = AddConst::new(dec_out, Complex::new(1.1, 2.0));
let sink = Box::new(NullSink::new(add_out));
let mut g = Graph::new();
g.add(Box::new(src));
g.add(Box::new(dec));
g.add(Box::new(add));
g.add(sink);
g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
pub struct Graph {
    spent_time: Option<std::time::Duration>,
    spent_cpu_time: Option<std::time::Duration>,
    blocks: Vec<Box<dyn Block>>,
    cancel_token: CancellationToken,
    times: Vec<std::time::Duration>,
    cpu_times: Vec<std::time::Duration>,
}

impl Graph {
    /// Create a new flowgraph.
    pub fn new() -> Self {
        Self {
            spent_time: None,
            spent_cpu_time: None,
            blocks: Vec::new(),
            times: Vec::new(),
            cpu_times: Vec::new(),
            cancel_token: CancellationToken::new(),
        }
    }
}

#[must_use]
pub(crate) fn get_cpu_time() -> std::time::Duration {
    use libc::{CLOCK_PROCESS_CPUTIME_ID, clock_gettime, timespec};
    // SAFETY: Zeroing out a timespec struct is just all zeroes.
    let mut ts: timespec = unsafe { std::mem::zeroed() };
    // SAFETY: Local variable written my C function.
    let rc = unsafe { clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    if rc != 0 {
        panic!("clock_gettime()");
    }
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

impl GraphRunner for Graph {
    fn add(&mut self, b: Box<dyn Block + Send>) {
        self.blocks.push(b);
    }

    // TODO: fix this so that Drop is run for blocks that EOF.
    fn run(&mut self) -> Result<()> {
        let st = Instant::now();
        let start_run_cpu = get_cpu_time();
        self.times
            .resize(self.blocks.len(), std::time::Duration::default());
        self.cpu_times
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
                let name = b.block_name().to_owned();
                let st = Instant::now();
                let st_cpu = get_cpu_time();
                let ret = b
                    .work()
                    .map_err(|e| Error::wrap(e, format!("in block {name}")))?;

                self.times[n] += st.elapsed();
                self.cpu_times[n] += get_cpu_time() - st_cpu;
                match ret {
                    BlockRet::Again => {
                        drop(ret);
                        // Block did something.
                        //trace!("… {} was not starved", b.block_name());
                        done = false;
                        all_idle = false;
                    }
                    BlockRet::Pending => {
                        done = false;
                    }
                    BlockRet::WaitForFunc(_) => {
                        drop(ret);
                        if b.eof() {
                            eof[n] = true;
                        }
                    }
                    BlockRet::WaitForStream(stream, _need) => {
                        let closed = stream.closed();
                        drop(ret);
                        if b.eof() || closed {
                            // TODO: This doesn't actually drop the block. Maybe
                            // self.blocks needs to contain `Option`s?
                            eof[n] = true;
                        }
                    }
                    BlockRet::EOF => {
                        eof[n] = true;
                    }
                };
                if eof[n] {
                    info!("{name} EOF, exiting");
                }
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
        self.spent_time = Some(st.elapsed());
        self.spent_cpu_time = Some(get_cpu_time() - start_run_cpu);
        for line in self
            .generate_stats()
            .expect("failed to generate stats after run")
            .split('\n')
        {
            if !line.is_empty() {
                info!("{line}");
            }
        }
        Ok(())
    }

    fn generate_stats(&self) -> Option<String> {
        let elapsed = self.spent_time?;
        let elapsed_cpu = self.spent_cpu_time?.as_secs_f64();
        let total = self
            .times
            .iter()
            .cloned()
            .sum::<std::time::Duration>()
            .as_secs_f64();
        let block_cpu = self
            .cpu_times
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

        let dashes = "-".repeat(ml + 46) + "\n";
        let (secw, secd) = (10, 3);
        let (pw, pd) = (7, 2);

        let mut s: String = format!(
            "{:<width$}    Seconds  Percent    CPU sec     CPU%   Mul\n",
            "Block name",
            width = ml
        );
        s.push_str(&dashes);
        for (n, b) in self.blocks.iter().enumerate() {
            s.push_str(&format!(
                "{:<width$} {:secw$.secd$} {:>pw$.pd$}% {:secw$.secd$} {:>pw$.pd$}% {:5.1}\n",
                b.block_name(),
                self.times[n].as_secs_f32(),
                100.0 * self.times[n].as_secs_f64() / total,
                self.cpu_times[n].as_secs_f32(),
                100.0 * self.cpu_times[n].as_secs_f64() / block_cpu,
                self.cpu_times[n].as_secs_f32() / self.times[n].as_secs_f32(),
                width = ml,
            ));
        }
        s.push_str(&dashes);
        s.push_str(&format!(
            "{:<width$} {total:secw$.secd$} {:>pw$.pd$}% {block_cpu:secw$.secd$} {:>pw$.pd$}% {:5.1}\n",
            "All blocks",
            100.0 * total / elapsed,
            100.0 * block_cpu / elapsed_cpu,
            block_cpu / elapsed,
            width = ml,
        ));
        s.push_str(&format!(
            "{:<width$} {:secw$.secd$} {:>pw$.pd$}% {:secw$.secd$} {:>pw$.pd$}% {:5.1}\n",
            "Non-block time",
            elapsed - total,
            100.0 * (elapsed - total) / elapsed,
            elapsed_cpu - block_cpu,
            100.0 * (elapsed_cpu - block_cpu) / elapsed_cpu,
            (elapsed_cpu - block_cpu) / (elapsed - total),
            width = ml,
        ));
        s.push_str(&format!(
            "{:<width$} {elapsed:secw$.secd$} {:>pw$.pd$}% {:secw$.secd$} {:>pw$.pd$}% {:5.1}\n",
            "Elapsed seconds",
            100.0,
            elapsed_cpu,
            100.0,
            elapsed_cpu / elapsed,
            width = ml,
        ));
        Some(s)
    }

    fn cancel_token(&self) -> CancellationToken {
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn small_test() -> Result<()> {
        use crate::Complex;
        use crate::blocks::{AddConst, FileSource, NullSink, RtlSdrDecode};
        let (src, src_out) = FileSource::new("/dev/null")?;
        let (dec, dec_out) = RtlSdrDecode::new(src_out);
        let (add, add_out) = AddConst::new(dec_out, Complex::new(1.1, 2.0));
        let sink = Box::new(NullSink::new(add_out));
        let mut g = Graph::new();
        g.add(Box::new(src));
        g.add(Box::new(dec));
        g.add(Box::new(add));
        g.add(sink);
        g.run()?;
        Ok(())
    }

    #[test]
    fn canceller() -> Result<()> {
        let cancel = CancellationToken::default();
        assert!(!cancel.is_canceled());
        cancel.cancel();
        assert!(cancel.is_canceled());
        Ok(())
    }

    #[test]
    fn default_graph() -> Result<()> {
        let g = Graph::default();
        let cancel = g.cancel_token();
        assert!(!cancel.is_canceled());
        Ok(())
    }
}
/* vim: textwidth=80
 */
