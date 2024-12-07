/*! Multithreaded version of Graph, otherwise the same as graph.rs.
 */
use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::Result;
use log::{debug, error, info, trace};

use crate::block::{Block, BlockRet};
use crate::graph::CancellationToken;

/**
A graph is a thing that RustRadio runs, to let blocks "talk to each
other" via streams.

# Example

```
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::Complex;
use rustradio::blocks::{FileSource,RtlSdrDecode,AddConst,NullSink};
let src = Box::new(FileSource::<u8>::new("/dev/null", false)?);
let dec = Box::new(RtlSdrDecode::new(src.out()));
let add = Box::new(AddConst::new(dec.out(), Complex::new(1.1, 2.0)));
let sink = Box::new(NullSink::new(add.out()));
let mut g = MTGraph::new();
g.add(src);
g.add(dec);
g.add(add);
g.add(sink);
g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
pub struct MTGraph {
    blocks: Vec<Box<dyn Block + Send>>,
    cancel_token: CancellationToken,
    times: BTreeMap<(usize, String), std::time::Duration>,
}

impl MTGraph {
    /// Create a new flowgraph.
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            times: BTreeMap::new(),
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
            let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, BlockRet)>(block_count);
            (std::thread::Builder::new()
             .name("exit monitor".to_string())
             .spawn(move || -> Result<()> {
                 let mut status = vec![BlockRet::Ok; block_count];

                 let mut first_phase = true;
                 while let Ok((index, s)) = rx.recv() {
                     // We'll skip deeper checks if we already by the
                     // received message know that we're not done.
                     let mut maybe_done = match s {
                         BlockRet::Ok | BlockRet::Pending |BlockRet::OutputFull => {
                             // Bump down to first phase, if not already there.
                             first_phase = true;
                             false
                         },
                         BlockRet::Noop | BlockRet::EOF => true,
                         BlockRet::InternalAwaiting => panic!("InternalAwaiting should never be received"),
                     };

                     // Update state.
                     status[index] = s;

                     if !maybe_done {
                         continue;
                     }

                     // Don't bother checking all states if we already know we're not done.
                     for si in &status {
                         match si {
                             BlockRet::Ok | BlockRet::Pending |BlockRet::OutputFull=> {
                                 trace!("MTGraph exit monitor: index {index} not done, has state {:?}", si);
                                 first_phase = true;
                                 maybe_done = false;
                                 break;
                             },
                             BlockRet::Noop |BlockRet::EOF => {},
                             BlockRet::InternalAwaiting => {
                                 maybe_done = false;
                                 // We can safely break here, without
                                 // checking for more Ok/Pending,
                                 // since the only way they could be
                                 // set to that is if we received a
                                 // message, and in that case we've
                                 // already been bumped down to the
                                 // first phase.
                                 break;
                             },
                         };
                     }

                     if maybe_done {
                         if !first_phase {
                             debug!("All blocks returning done in two phases.");
                             break;
                         }
                         debug!("First phase of done detection completed. Resetting for second phase.");
                         first_phase = false;
                         for si in &mut status {
                             if !matches![si, BlockRet::EOF] {
                                 *si = BlockRet::InternalAwaiting;
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
        let mut threads = Vec::new();
        let mut index = self.blocks.len();
        while let Some(mut b) = self.blocks.pop() {
            index -= 1;
            let cancel_token = self.cancel_token.clone();
            let em_tx = em_tx.clone();
            debug!("Starting thread {}", b.block_name());
            let th = std::thread::Builder::new()
                .name(b.block_name().to_string())
                .spawn(move || -> Result<std::time::Duration> {
                    let idle_sleep = std::time::Duration::from_millis(1);
                    let mut tt = std::time::Duration::new(0, 0);
                    while !cancel_token.is_canceled() {
                        let st = Instant::now();
                        let ret = b.work()?;
                        tt += st.elapsed();
                        em_tx
                            .send((index, ret.clone()))
                            .expect("mpsc status send failed");
                        match ret {
                            BlockRet::Ok => {}
                            BlockRet::EOF => {
                                return Ok(tt);
                            }
                            BlockRet::Noop | BlockRet::OutputFull => {
                                std::thread::sleep(idle_sleep);
                            }
                            BlockRet::Pending => {
                                std::thread::sleep(idle_sleep);
                            }
                            BlockRet::InternalAwaiting => {
                                panic!("blocks must never return InternalAwaiting")
                            }
                        }
                    }
                    Ok(tt)
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
            self.times.insert((n, name), j);
        }
        exit_monitor.join().unwrap().unwrap();
        for line in self.generate_stats(st.elapsed()).split('\n') {
            if !line.is_empty() {
                info!("{}", line);
            }
        }
        Ok(())
    }

    /// Return a string with stats about where time went.
    fn generate_stats(&self, elapsed: std::time::Duration) -> String {
        let total = self
            .times
            .values()
            .sum::<std::time::Duration>()
            .as_secs_f64();
        let names: Vec<String> = self
            .times
            .keys()
            .map(|(n, name)| format!("{}/{}", name, n))
            .collect();
        let ml = names.iter().map(|b| b.len()).max().unwrap(); // unwrap: can only fail if block list is empty.
        let ml = std::cmp::max(ml, "Elapsed seconds".len());
        let elapsed = elapsed.as_secs_f64();

        let dashes = "-".repeat(ml + 20) + "\n";
        let (secw, secd) = (10, 3);
        let (pw, pd) = (7, 2);

        let mut s: String = format!("{:<width$}    Seconds  Percent\n", "Block name", width = ml);
        s.push_str(&dashes);
        for (n, tt) in self.times.values().enumerate() {
            let name = &names[n];
            s.push_str(&format!(
                "{:<width$} {:secw$.secd$} {:>pw$.pd$}%\n",
                name,
                tt.as_secs_f32(),
                100.0 * tt.as_secs_f64() / total,
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
