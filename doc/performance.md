# Performance

RustRadio, unlike GNU Radio, does not choose inner loops or "kernels" at
runtime. Instead it assumes that a binary is already optimized for the
appropriate platform.

This is so that all code gets correctly optimized, not just those parts that are
deemed to be high level kernels. Let the compiler do its job. It'll vectorize
and optimize more than you'd think.

## Optimize for your platform

Get your architecture tuple:

```
$ cargo +nightly rustc -Z unstable-options --print host-tuple
x86_64-unknown-linux-gnu
```

Then configure cargo to build for the local machine by adding something like
this to `~/.cargo/config.toml`.

```
[target.x86_64-unknown-linux-gnu]
rustflags = ["-Ctarget-cpu=native"]
```

## Profile guided optimizations

```
# Build binary with profile instrumentation.
RUSTFLAGS="-Cprofile-generate=./profile-data -Ctarget-cpu=native" cargo build --release

# Run the binary with typical input. This will be much slower than usual.
./target/release/yourbinary

# Merge profile data.
llvm-profdata-19 merge -o merged.profdata ./profile-data

# Build again, but using the profile data.
RUSTFLAGS="-Cprofile-use=$(pwd)/merged.profdata -Ctarget-cpu=native" cargo build --release

# Now your newly built binary should be faster. YMMV.
```

## Benchmarking

A flow graph will have many moving parts, so one way to benchmark is to run it
on example data. For audio domain AX.25 there's a WA8LMF CD. For others, and
especially for your own domain, you may need to make your own.

To benchmark blocks or other RustRadio components, see the `benches` directory,
which can then be run with:

```
cargo +nightly bench
```

## Flamegraphs

First install flamegraph:

```
cargo install flamegraph
```

Then generate one:

```
cargo +nightly flamegraph \
    --features rtlsdr,fast-math,simd \
    --release \
    --example ax25-1200-rx \
    -- \
    -a \
    -r cd/cd_tracks_01.au \
    --sample_rate 44100 \
    -o tmp
```

## Block optimizations

Ideas for how to make your block faster.

### Parallelize

Rayon makes it easy to parallelize a lot of cases. Simply replacing `iter_mut()`
with `par_iter_mut()` in the `Hilbert` block made it 4-5x faster in real time.

### Other tips

* Don't call `.read_buf()` or `.write_buf()` more than necessary.
* Don't call `.slice()` on the buffers too often either.
* Rust is pretty good at vectorizing simple loops, as long as you enabled
  `target-cpu=native` (see above), so you may not need to bother doing it
  manually. Check the assembly.
  * If you do want to do it, `FIR` has a `std::simd` and AVX2 specialization,
    which can serve as inspiration.
