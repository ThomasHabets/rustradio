# Writing a new block

For most uses of SDR beyond "hello world" (which is often an FM radio receiver),
you'll have to write your own block.

## First step: do I need a block?

For simple 1:1 transformations, you can use a `Map` block. For example
converting from `Complex` to the real part can be done using:

```
    MapBuilder::new(prev_stream, |x| x.re)
        .name("ComplexToReal".to_owned())
        .build()
```

But eventually you'll need to write a real block.

## Second: Prefer synchronous (sync) blocks

Sync blocks produce exactly one output sample per input sample. This makes the
API very simple.

If tags should just be passed through as-is, it's even simpler.

Sync blocks can have multiple inputs and outputs, but every operation consumes
exactly one sample from each input, and produces one sample on each output.

```
#[derive(rustradio_macros::Block)]
#[rustradio(new, sync)]
pub struct AddSub {
    #[rustradio(in)]
    a: ReadStream<Float>,

    #[rustradio(in)]
    b: ReadStream<Float>,

    #[rustradio(out)]
    sum: WriteStream<Float>,

    #[rustradio(out)]
    diff: WriteStream<Float>,
}
impl AddSub {
    fn process_sync(&self, a: Float, b: Float) -> (Float, Float) {
        (a + b, a - b)
    }
}
```

## The block `new()` function

All blocks will very likely derive `rustradio_macros::Block`. That allows
tagging input and output streams.

This derive may become mandatory in the future.

The derive macro supports generating `new()`. For this generated constructor,
input streams are passed to the block's `new()` function. After the input
streams, it needs the untagged fields passed in. The function returns the
created block, and all output fields.

This means that the input and output streams are part of the block's API.

Even if a block implements `new()` manually instead of generating it, it's
expected to follow this standard.

Fields can be default-created instead of passed in, by tagging them with
`#[rustradio(default)]`.

TODO: document optional input and output streams.

## The block `work()` function

For general blocks, the core of a block is its `work()` function.

```
impl Block for MyBlock {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (input_buffer, input_tags) = self.src.read_buf();
        let mut output_stream = self.dst.write_buf()?;
        let output_slice = output_stream.slice();
        let max_output_samples = output_slice.len();
        let (mydata, input_used) = [… do something with input_buffer and max_output_samples …];
        let out_len = mydata.len();
        output_slice[..out_len].copy_from_slice(&mydata);

        input_buffer.consume(input_used);
        output_stream.produce(mydata.len());
        if need_more_input {
            Ok(BlockRet::WaitForStream(&self.src, how_much_more))
        } else if need_more_output_space {
            Ok(BlockRet::WaitForStream(&self.dst, space_needed))
        } else {
            // I guess I have both more input and output.
            // Call me again, immediately. I'm ready.
            Ok(BlockRet::Again)
        }
    }
}
```

TODO:
* NoCopy blocks.

## A block finishing

## SIMD

If manual vectorization is needed for blocks in the main RustRadio library,
prefer `std::simd` over `core::arch`. As of this writing `std::simd` is only
supported in `nightly` Rust, but it'll help every future architecture and
instruction set.

For more performance, or for performance boost also with `stable` Rust, you can
*also* add a vector implementation using `core::arch::*`.

If using `core::arch`, only skip the `std::simd` implementation if benchmark it
to not help over the fallback implementation.

Once `std::simd` is in `stable`, it'll be fine to skip the default
implementation. `std::simd` can help generate code faster than regular
Rust even on non-vector capable hardware.
