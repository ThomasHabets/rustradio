//! Derive macros for rustradio.
//!
//! Most blocks should derive from `Block`.

/// Block derive macro.
///
/// Most blocks should derive from this macro. Example use:
///
/// ```
/// use rustradio::{Result, Error};
/// use rustradio::block::{Block, BlockRet};
/// use rustradio::stream::{ReadStream, WriteStream};
/// #[derive(rustradio_macros::Block)]
/// #[rustradio(new)]
/// pub struct MyBlock<T: Copy + Send + Sync> {
///   #[rustradio(in)]
///   src: ReadStream<T>,
///   #[rustradio(out)]
///   dst: WriteStream<T>,
///
///   other_parameter: u32,
/// }
/// impl<T: Copy + Send + Sync> Block for MyBlock<T> {
///   fn work(&mut self) -> Result<BlockRet> {
///     todo!()
///   }
/// }
/// ```
///
/// Struct attributes:
/// * `new`: Generate `new()`, taking input streams and other args.
/// * `out`: Generate `out()`, returning all output streams.
/// * `crate`: Block is in the main Rustradio crate.
/// * `sync`: Block is "one in, one out" via `process_sync()` instead of
///   `work()`.
/// * `sync_tag`: Same as `sync`, but allow tag processing using
///   `process_sync_tags()`.
/// * `sync_nocopy_tag`: Same as `sync_tag`, but for nocopy streams.
/// * `custom_name`: Call `custom_name()` instead of using the struct name, as
///   name.
/// * `noeof`: Don't generate `eof()` logic.
/// * `bound`: Add more trait bound strings that should apply to impl.
///
/// Field attributes:
/// * `in`: Input stream.
/// * `out`: Output stream.
/// * `default`: Skip this field as arg for the `new()` function, and instead
///   default it.
/// * `into`: When the `new()` function is generated, let non-stream values
///   accept anything `.into()`-convertable into the given type, not just the
///   generated type directly.
///
/// ## Sync blocks
///
/// A block using the `sync` attribute does not implement the standard `work()`
/// function, but instead implements `process_sync()`. This greatly simplifies
/// the API, at the cost of only being able to process samples one-for-one, with
/// no history.
///
/// `process_sync()` takes one sample from each `in` stream, and returns one
/// value for each `out` stream.
///
/// All output streams get the tags from the *first* input stream. For more
/// control of tag propagation, use `sync_tag` instead.
///
/// For an example of a sync block taking two input streams and producing one
/// output stream, see the `Add` block. Repeated here in simplified form:
///
/// ```rust
/// use rustradio::Float;
/// use rustradio::stream::{ReadStream, WriteStream};
/// #[derive(rustradio_macros::Block)]
/// #[rustradio(new, sync)]
/// pub struct Add {
///     #[rustradio(in)]
///     a: ReadStream<Float>,
///     #[rustradio(in)]
///     b: ReadStream<Float>,
///     #[rustradio(out)]
///     dst: WriteStream<Float>,
/// }
///
/// impl Add {
///     fn process_sync(&self, a: Float, b: Float) -> Float {
///         a + b
///     }
/// }
/// ```
///
/// ## Tags on a sync block
///
/// A `sync_tag` is like a `sync` block but takes control of tag propagation.
/// This enables:
/// * Reading of tags.
/// * Writing of tags (e.g. `BurstTagger` and `CorrelateAccessCode` blocks).
/// * Using different tags for different output streams.
///
/// A version of `Tee` that only writes tags to the first stream could be:
///
/// ```rust
/// use std::borrow::Cow;
///
/// use rustradio::Float;
/// use rustradio::stream::{ReadStream, WriteStream, Tag};
/// #[derive(rustradio_macros::Block)]
/// #[rustradio(new, sync_tag)]
/// struct Tee {
///     #[rustradio(in)]
///     src: ReadStream<Float>,
///     #[rustradio(out)]
///     dst1: WriteStream<Float>,
///     #[rustradio(out)]
///     dst2: WriteStream<Float>,
/// }
///
/// impl Tee {
///     fn process_sync_tags<'a>(
///         &self,
///         s: Float,
///         ts: &'a [Tag],
///     ) -> (Float, Cow<'a, [Tag]>, Float, Cow<'a, [Tag]>) {
///         (s, Cow::Borrowed(ts), s, Cow::Owned(vec![]))
///     }
/// }
/// ```
///
/// But a better solution for that case would likely be to use regular `Tee`,
/// and then have a second block filter the tags of the second stream.
#[proc_macro_derive(Block, attributes(rustradio))]
pub fn derive_block(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    rustradio_macros_code::derive_block(input.into()).into()
}
/* vim: textwidth=80
 */
