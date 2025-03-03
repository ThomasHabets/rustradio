//! RustRadio Block implementation
//!
//! Blocks are the main building blocks of rustradio. They each do one
//! thing, and you connect them together with streams to process the data.
use anyhow::Result;

use crate::Error;
use crate::stream::StreamWait;

/// Return type for all blocks.
///
/// This will let the scheduler know if more data could come out of this block,
/// or if it should just never bother calling it again.
//#[derive(Debug)]
pub enum BlockRet<'a> {
    /// Everything is fine, but no information about when more data could be
    /// created.
    ///
    /// The graph scheduler should feel free to call the `work` function again
    /// without waiting or sleeping.
    ///
    /// Ok should not be returned for "polling". In other words, it should not
    /// be returned repeatedly without data being consumed or produced.
    ///
    /// Good examples of returning Ok:
    /// * A block finished being in a state (e.g. writing headers), and does not
    ///   want to deal with restarting `work()` under the new state. Next time
    ///   `work()` is called, it'll be in a new state, so it's just temporary.
    ///   Example `AuEncode`.
    /// * Stream status is checked at the start of `work()`, so instead of
    ///   re-checking status after a `produce()`/`consume()`, it's easier
    ///   to just let the graph call `work()` again.
    ///   Examples: `RtlSdrDecode` and `FirFilter`.
    ///
    /// Importantly, in both these examples, a second `work()` call is not
    /// expected to do nothing, and just return `Ok`. It'll either do useful
    /// work, or it'll properly return a status showing what it's blocked on.
    Ok,

    /// Block didn't produce anything this time, but has a background
    /// process that may suddenly produce.
    ///
    /// The difference between `Ok` and `Pending` is that `Pending` implies to
    /// the graph runner that it's reasonable to sleep a bit before calling
    /// `work` again. And that activity on any stream won't help either way.
    ///
    /// Example: `RtlSdrSource` may not currently have any new data, but we
    /// can't control when it does.
    Pending,

    /// Block indicates that there's no point calling it until the provided
    /// function has been run.
    ///
    /// The function is blocking, but should not block for "too long", since it
    /// prevents checking for exit conditions like stream EOF and Ctrl-C.
    ///
    /// For a single threaded graph, it would stall all blocks, so it's not
    /// called at all, and thus becomes equivalent to returning `Ok`.
    ///
    /// Discouraged: Prefer WaitForStream when possible.
    WaitForFunc(Box<dyn Fn() + 'a>),

    /// Signal that we're waiting for a stream. Either an input or output
    /// stream.
    ///
    /// This is preferred over `WaitForFunc`, since graph executors know more
    /// about the stream. E.g. if a block says that it's waiting for more data
    /// on a stream, and the stream writer side goes away, then the waiting
    /// block will never be satisfied, and is therefore also shut down.
    WaitForStream(&'a dyn StreamWait, usize),

    /// Block indicates that it will never produce more input.
    ///
    /// Examples:
    /// * Reading from file, without repeating, and file reached EOF.
    /// * Reading from a `VectorSource` that reached its end.
    /// * Head block reached its max.
    EOF,
}

/// Provide name of block.
///
/// This has to be a separate trait, because often the `impl` is proc macro
/// generated, and it's not possible to re-open the same trait `impl` in Rust.
pub trait BlockName {
    /// Name of block
    ///
    /// Not name of *instance* of block. But it may include the type. E.g.
    /// `FileSource<Float>`.
    fn block_name(&self) -> &str;
}

/// Enable asking if a block is done, and will never return any more data.
///
/// This has to be a separate trait, because often the `impl` is proc macro
/// generated, and it's not possible to re-open the same trait `impl` in Rust.
pub trait BlockEOF {
    /// Return EOF status.
    ///
    /// Mutable because if eof, the block is also responsible setting EOF on its
    /// output streams.
    fn eof(&mut self) -> bool {
        false
    }
}

/// Block trait. Must be implemented for all blocks.
///
/// Simpler blocks can use macros to avoid needing to implement `work()`.
pub trait Block: BlockName + BlockEOF {
    /// Block work function
    ///
    /// A block implementation keeps track of its own inputs and outputs.
    fn work(&mut self) -> Result<BlockRet, Error>;
}
/* vim: textwidth=80
 */
