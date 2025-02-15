/*! RustRadio Block implementation

Blocks are the main building blocks of rustradio. They each do one
thing, and you connect them together with streams to process the data.

*/

use anyhow::Result;

use crate::Error;

/** Return type for all blocks.

This will let the scheduler know if more data could come out of this block, or if
it should just never bother calling it again.

TODO: Add state for "don't call me unless there's more input".
 */
//#[derive(Debug)]
pub enum BlockRet<'a> {
    /// At least one sample was produced.
    ///
    /// More data may be produced only if more data comes in.
    ///
    /// Ideally the difference between Noop and Ok would be inferred, but since
    /// the input and output streams are owned by the block, we don't yet see
    /// that.
    Ok,

    /// Block didn't produce anything this time, but has a background
    /// process that may suddenly produce.
    Pending,

    /// Produced nothing, because not enough input.
    ///
    /// When all nodes in a graph produce either EOF or Noop, the graph is
    /// considered done, and the `g.run()` returns.
    Noop,

    /// Waiting for more input or more output space.
    ///
    /// The function is blocking.
    /// TODO: but it can't block forever.
    WaitForStream(Box<dyn Fn() + 'a>),

    /// Produced nothing, because not enough output space.
    OutputFull,

    // More data may be produced even if no more data comes in.
    // Currently not used.
    // Background,
    /// Block indicates that it will never produce more input.
    ///
    /// Examples:
    /// * reading from file, without repeating, and file reached EOF.
    /// * Head block reached its max.
    EOF,

    /// Internal state for two-phase done-detection.
    InternalAwaiting,
}

pub trait BlockName {
    /// Name of block
    ///
    /// Not name of *instance* of block. But it may include the type. E.g.
    /// `FileSource<Float>`.
    fn block_name(&self) -> &str;
}

pub trait BlockEOF {
    /// Return EOF status.
    ///
    /// Mutable because if eof, the block is also responsible setting EOF on its
    /// output streams.
    fn eof(&mut self) -> bool {
        false
    }
}

/// Block trait, that must be implemented for all blocks.
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
