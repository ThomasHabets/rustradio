/*! RustRadio Block implementation

Blocks are the main building blocks of rustradio. They each do one
thing, and you connect them together with streams to process the data.

*/

use anyhow::Result;

use crate::stream::StreamWait;
use crate::Error;

/** Return type for all blocks.

This will let the scheduler know if more data could come out of this block, or if
it should just never bother calling it again.

TODO: Add state for "don't call me unless there's more input".
 */
//#[derive(Debug)]
pub enum BlockRet<'a> {
    /// Everything is fine, but no information about when more data could be
    /// created.
    ///
    /// TODO: This and Pending should work things out between each other.
    Ok,

    /// Block didn't produce anything this time, but has a background
    /// process that may suddenly produce.
    Pending,

    /// Block indicates that there's no point calling it until the provided
    /// function has been run.
    ///
    /// The function is blocking, but should not block for "too long", since it
    /// prevents checking for exit conditions like stream EOF and Ctrl-C.
    ///
    /// For a single threaded graph, it would stall all blocks, so it's not
    /// used.
    WaitForFunc(Box<dyn Fn() + 'a>),

    /// Signal that we're waiting for a stream. Either an input or output
    /// stream.
    ///
    /// This is basically the same as WaitForFunc, but with a simpler API.
    WaitForStream(&'a dyn StreamWait, usize),

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
