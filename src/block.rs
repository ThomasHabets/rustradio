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
#[derive(Debug, Clone)]
pub enum BlockRet {
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

/** Macro to make it easier to write converting blocks with tags.

Output may will be different from input type.

`process_one(&mut self, s: Type1) -> Type2` must be implemented by the
block.

Both types are derived, so only the name of the block is needed at
macro call.

Example block using this: `FloatToU32`.

Both types are derived, so only the name of the block is needed at
macro call.
*/
#[macro_export]
macro_rules! map_block_convert_tag_macro {
    ($name:path, $out:ident) => {
        impl $name {
            /// Return the output stream.
            pub fn out(&self) -> Streamp<$out> {
                self.dst.clone()
            }
        }

        impl $crate::block::BlockName for $name {
            fn block_name(&self) -> &str {
                stringify! {$name}
            }
        }
        impl $crate::block::Block for $name {
            fn work(&mut self) -> Result<$crate::block::BlockRet, $crate::Error> {
                // Bindings, since borrow checker won't let us call
                // mut `process_one` if we borrow `src` and `dst`.
                let ibind = self.src.clone();
                let obind = self.dst.clone();

                // Get input and output buffers.
                let (i, _itags) = ibind.read_buf()?;
                let mut o = obind.write_buf()?;

                // Don't process more than we have, and fit.
                let n = std::cmp::min(i.len(), o.len());
                if n == 0 {
                    return Ok($crate::block::BlockRet::Noop);
                }

                let mut otags = Vec::new();
                // Map one sample at a time. Is this really the best way?
                for (n, (place, ival)) in o.slice().iter_mut().zip(i.iter()).enumerate() {
                    let (t, tags) = self.process_one(*ival, &[]);
                    *place = t;
                    for tag in tags {
                        otags.push(Tag::new(n, tag.key().into(), tag.val().clone()));
                    }
                }

                // Finalize.
                o.produce(n, &otags);
                i.consume(n);
                Ok($crate::block::BlockRet::Ok)
            }
        }
    };
}
/* vim: textwidth=80
 */
