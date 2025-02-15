/*! Multithreaded version of Graph, otherwise the same as graph.rs.
 */
use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::Result;
use log::{debug, error, info, trace};

use crate::block::{Block, BlockRet};
use crate::graph::{get_cpu_time, CancellationToken};

#[derive(Default, Debug)]
struct BlockStats {
    elapsed: std::time::Duration,
    work_calls: usize,
}
/**
A graph is a thing that RustRadio runs, to let blocks "talk to each
other" via streams.

# Example

```
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::Complex;
use rustradio::blocks::{FileSource,RtlSdrDecode,AddConst,NullSink};
let (src, prev) = FileSource::<u8>::new("/dev/null", false)?;
let (dec, prev) = RtlSdrDecode::new(prev);
let (add, prev) = AddConst::new(prev, Complex::new(1.1, 2.0));
let sink = NullSink::new(prev);
let mut g = MTGraph::new();
g.add(Box::new(src));
g.add(Box::new(dec));
g.add(Box::new(add));
g.add(Box::new(sink));
g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
pub struct MTGraph {
    spent_time: Option<std::time::Duration>,
    spent_cpu_time: Option<std::time::Duration>,
    blocks: Vec<Box<dyn Block + Send>>,
    cancel_token: CancellationToken,
    block_stats: BTreeMap<(usize, String), BlockStats>,
}

impl MTGraph {
    /// Create a new flowgraph.
    pub fn new() -> Self {
        Self {
            spent_time: None,
            spent_cpu_time: None,
            blocks: Vec::new(),
            block_stats: BTreeMap::new(),
            cancel_token: CancellationToken::new(),
        }
    }
}

impl crate::graph::GraphRunner for MTGraph {
    /// Add a block to the flowgraph.
    fn add(&mut self, b: Box<dyn Block + Send>) {
        self.blocks.push(b);
    }

    /// Run the graph until completion.
    fn run(&mut self) -> Result<()> {
        let (exit_monitor, em_tx) = {
            let cancel_token = self.cancel_token.clone();
            let block_count = self.blocks.len();
            let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, bool)>(block_count);
            (std::thread::Builder::new()
             .name("exit monitor".to_string())
             .spawn(move || -> Result<()> {
                 let mut status = vec![false; block_count];

                 let mut first_phase = true;
                 while let Ok((index, mut maybe_done)) = rx.recv() {
                     // We'll skip deeper checks if we already by the
                     // received message know that we're not done.
                     if !maybe_done {
                         first_phase = true;
                     }

                     // Update state.
                     status[index] = maybe_done;

                     if !maybe_done {
                         continue;
                     }

                     // Don't bother checking all states if we already know we're not done.
                     for si in &status {
                         if !si {
                             trace!("MTGraph exit monitor: index {index} not done, has state {:?}", si);
                             first_phase = true;
                             maybe_done = false;
                         }
                     }

                     if maybe_done {
                         if !first_phase {
                             debug!("All blocks returning done in two phases.");
                             break;
                         }
                         debug!("First phase of done detection completed. Resetting for second phase.");
                         first_phase = false;
                         for si in &mut status {
                             if *si {
                                 *si = true;
                             }
                         }
                     }
                 }
                 // Cancel all remaining blocks.
                 cancel_token.cancel();

                 // Discard remaining messages. This saves the sender getting an error on send.
                 while rx.recv().is_ok() {}
                 Ok(())
                })?, tx)
        };

        let st = Instant::now();
        let run_start_cpu = get_cpu_time();
        let mut threads = Vec::new();
        let mut index = self.blocks.len();
        while let Some(mut b) = self.blocks.pop() {
            index -= 1;
            let cancel_token = self.cancel_token.clone();
            let em_tx = em_tx.clone();
            debug!("Starting thread {}", b.block_name());
            let th = std::thread::Builder::new()
                .name(b.block_name().to_string())
                .spawn(move || -> Result<BlockStats> {
                    let idle_sleep = std::time::Duration::from_millis(1);
                    let mut stats = BlockStats::default();
                    while !cancel_token.is_canceled() {
                        let st = Instant::now();
                        let ret = match b.work() {
                            Ok(v) => v,
                            Err(e) => {
                                error!("Block work function failed: {e}");
                                return Err(e.into());
                            }
                        };
                        stats.work_calls += 1;
                        let ret = b.work()?;
                        stats.elapsed += st.elapsed();
                        let maybe_done = match ret {
                            BlockRet::Ok | BlockRet::Pending | BlockRet::OutputFull => {
                                // Bump down to first phase, if not already there.
                                false
                            }
                            BlockRet::Noop | BlockRet::EOF => true,
                            BlockRet::WaitForStream(_) => true,
                            BlockRet::InternalAwaiting => {
                                panic!("InternalAwaiting should never be received")
                            }
                        };
                        em_tx
                            .send((index, maybe_done))
                            .expect("mpsc status send failed");
                        match ret {
                            BlockRet::Ok => {}
                            BlockRet::EOF => {
                                return Ok(stats);
                            }
                            BlockRet::Noop | BlockRet::OutputFull => {
                                std::thread::sleep(idle_sleep);
                            }
                            BlockRet::WaitForStream(f) => {
                                f();
                            }
                            BlockRet::Pending => {
                                std::thread::sleep(idle_sleep);
                            }
                            BlockRet::InternalAwaiting => {
                                panic!("blocks must never return InternalAwaiting")
                            }
                        }
                    }
                    Ok(stats)
                });
            let th = match th {
                Err(x) => {
                    error!("Failed to spawn block thread: {:?}", x);
                    self.cancel_token.cancel();
                    break;
                }
                Ok(x) => x,
            };
            threads.push(th);
        }
        drop(em_tx);
        debug!("Joining threads");
        for (n, th) in threads.into_iter().rev().enumerate() {
            let name = th.thread().name().unwrap().to_string();
            debug!("Waiting for {}", name);
            let j = th
                .join()
                .expect("joining thread")
                .expect("block exit status");
            debug!("Thread {} finished with {:?}", name, j);
            self.block_stats.insert((n, name), j);
        }
        exit_monitor.join().unwrap().unwrap();
        self.spent_time = Some(st.elapsed());
        self.spent_cpu_time = Some(get_cpu_time() - run_start_cpu);
        for line in self.generate_stats().expect("can't happen").split('\n') {
            if !line.is_empty() {
                info!("{}", line);
            }
        }
        Ok(())
    }

    /// Return a string with stats about where time went.
    ///
    /// MTGraph can't measure per block CPU time, since rayon and other block
    /// threading is not measurable.
    fn generate_stats(&self) -> Option<String> {
        let elapsed = self.spent_time?.as_secs_f64();
        let elapsed_cpu = self.spent_cpu_time?.as_secs_f64();
        let total = self
            .block_stats
            .values()
            .map(|b| b.elapsed)
            .sum::<std::time::Duration>()
            .as_secs_f64();
        let names: Vec<String> = self
            .block_stats
            .keys()
            .map(|(n, name)| format!("{}/{}", name, n))
            .collect();
        let ml = names.iter().map(|b| b.len()).max().unwrap(); // unwrap: can only fail if block list is empty.
        let ml = std::cmp::max(ml, "Elapsed seconds".len());

        let dashes = "-".repeat(ml + 52) + "\n";
        let (secw, secd) = (10, 3);
        let (pw, pd) = (7, 2);

        let mut s: String = format!(
            "{:<width$}    Seconds  Percent     CPU sec   CPU%    Mul  Work\n",
            "Block name",
            width = ml
        );
        s.push_str(&dashes);
        for (n, stats) in self.block_stats.values().enumerate() {
            let tt = stats.elapsed;
            let name = &names[n];
            s.push_str(&format!(
                "{name:<width$} {:secw$.secd$} {:>pw$.pd$}%                         {:7}\n",
                tt.as_secs_f32(),
                100.0 * tt.as_secs_f64() / total,
                stats.work_calls,
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
        // This is nonsetse data at the moment. Skip it.
        s.push_str(&format!(
            "{:<width$} {:secw$.secd$} {:>pw$.pd$}%\n",
            "Non-block time",
            elapsed - total,
            100.0 * (elapsed - total) / elapsed,
            width = ml,
        ));
        s.push_str(&format!(
            "{:<width$} {elapsed:secw$.secd$} {:>pw$.pd$}% {elapsed_cpu:secw$.secd$} {:>pw$.pd$}% {:5.1}\n",
            "Elapsed seconds",
            100.0,
            100.0,
            elapsed_cpu / elapsed,
            width = ml,
        ));
        Some(s)
    }

    /// Return a cancellation token, for asynchronously stopping the
    /// graph, for example if the user presses Ctrl-C.
    ///
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rustradio::graph::GraphRunner;
    /// let mut g = rustradio::mtgraph::MTGraph::new();
    /// let cancel = g.cancel_token();
    /// ctrlc::set_handler(move || {
    ///     cancel.cancel();
    /// }).expect("failed to set Ctrl-C handler");
    /// g.run()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

impl Default for MTGraph {
    fn default() -> Self {
        Self::new()
    }
}
/* vim: textwidth=80
 */
