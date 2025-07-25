/*! Multithreaded version of Graph, otherwise the same as graph.rs.
 */
use std::collections::BTreeMap;
use std::time::Instant;

use crate::Result;
use log::{debug, error, info};

use crate::block::{Block, BlockRet};
use crate::graph::{CancellationToken, get_cpu_time};

const MIN_IDLE_SLEEP: std::time::Duration = std::time::Duration::from_millis(1);
const MAX_IDLE_SLEEP: std::time::Duration = std::time::Duration::from_millis(100);

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
let (src, prev) = FileSource::new("/dev/null")?;
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
    #[must_use]
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
    fn add(&mut self, b: Box<dyn Block + Send>) {
        self.blocks.push(b);
    }

    fn run(&mut self) -> Result<()> {
        let st = Instant::now();
        let run_start_cpu = get_cpu_time();
        let mut threads = Vec::new();
        while let Some(mut b) = self.blocks.pop() {
            let cancel_token = self.cancel_token.clone();
            debug!("Starting thread {}", b.block_name());
            let th = std::thread::Builder::new()
                .name(b.block_name().to_string())
                .spawn(move || -> Result<BlockStats> {
                    let name = b.block_name().to_string();
                    let mut idle_sleep = MIN_IDLE_SLEEP;
                    let mut stats = BlockStats::default();
                    while !cancel_token.is_canceled() {
                        let st = Instant::now();
                        stats.work_calls += 1;
                        let ret = match b.work() {
                            Ok(v) => v,
                            Err(e) => {
                                error!("Block work function for {name} failed: {e}");
                                return Err(e);
                            }
                        };
                        stats.elapsed += st.elapsed();
                        match ret {
                            BlockRet::Again => idle_sleep = MIN_IDLE_SLEEP,
                            BlockRet::EOF => {
                                break;
                            }
                            BlockRet::WaitForStream(stream, need) => {
                                let eof = stream.wait(need);
                                drop(ret);
                                if b.eof() || eof {
                                    break;
                                }
                            }
                            BlockRet::WaitForFunc(ref f) => {
                                f();
                                drop(ret);
                                if b.eof() {
                                    break;
                                }
                            }
                            BlockRet::Pending => {
                                std::thread::sleep(idle_sleep);
                                idle_sleep *= 2;
                                if idle_sleep > MAX_IDLE_SLEEP {
                                    idle_sleep = MAX_IDLE_SLEEP;
                                }
                            }
                        }
                    }
                    info!("Block {} done", b.block_name());
                    Ok(stats)
                });
            let th = match th {
                Err(x) => {
                    error!("Failed to spawn block thread: {x:?}");
                    self.cancel_token.cancel();
                    break;
                }
                Ok(x) => x,
            };
            threads.push(th);
        }
        debug!("Joining threads");
        for (n, th) in threads.into_iter().rev().enumerate() {
            let name = th.thread().name().unwrap().to_string();
            debug!("Waiting for {name}");
            let j = th
                .join()
                .expect("joining thread")
                .expect("block exit status");
            debug!("Thread {name} finished with {j:?}");
            self.block_stats.insert((n, name), j);
        }
        self.spent_time = Some(st.elapsed());
        self.spent_cpu_time = Some(get_cpu_time() - run_start_cpu);
        for line in self.generate_stats().expect("can't happen").split('\n') {
            if !line.is_empty() {
                info!("{line}");
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
            .map(|(n, name)| format!("{name}/{n}"))
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

    fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

impl Default for MTGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::graph::GraphRunner;
    use crate::stream::{Tag, TagValue};
    use crate::tests::assert_almost_equal_complex;
    use crate::{Complex, Float};

    #[test]
    fn small_test() -> Result<()> {
        use crate::blocks::{AddConst, FloatToComplex, VectorSink, VectorSource, add_const};
        let (src1, src1_out) = VectorSource::new(vec![2.0 as Float]);
        let (src2, src2_out) = VectorSource::new(vec![-1.0 as Float]);
        let (conv, conv_out) = FloatToComplex::new(src1_out, src2_out);
        let (add1, add1_out) = AddConst::new(conv_out, Complex::new(1.1, 2.0));
        let (add2, add2_out) = add_const(add1_out, Complex::new(1.3, -10.0));
        let sink = VectorSink::new(add2_out, 100);
        let hook = sink.hook();
        let mut g = MTGraph::new();
        g.add(Box::new(src1));
        g.add(Box::new(src2));
        g.add(Box::new(conv));
        g.add(Box::new(add1));
        g.add(Box::new(add2));
        g.add(Box::new(sink));
        g.run()?;
        assert_almost_equal_complex(hook.data().samples(), &[Complex::new(4.4, -9.0)]);
        assert_eq!(
            hook.data().tags(),
            &[
                Tag::new(0, "VectorSource::start", TagValue::Bool(true)),
                Tag::new(0, "VectorSource::repeat", TagValue::U64(0)),
                Tag::new(0, "VectorSource::first", TagValue::Bool(true)),
            ]
        );
        Ok(())
    }

    #[test]
    fn default_graph() -> Result<()> {
        let g = MTGraph::default();
        let cancel = g.cancel_token();
        assert!(!cancel.is_canceled());
        Ok(())
    }
}
/* vim: textwidth=80
 */
