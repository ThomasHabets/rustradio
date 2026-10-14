# Rust Radio

A library for digital signals processing in the spirit of GNU Radio.

* <https://github.com/ThomasHabets/rustradio>
* <https://crates.io/crates/rustradio>

## Differences from GNU Radio

### Pro

* Written in Rust instead of C++ & Python.
  * Easier to get things right than C++.
  * More performant than Python (and possibly more performant than C++).
  * Easier to ship as a built binary.
* Type safe streams.

### Con

* GNU Radio is obviously way more mature.
* GNU Radio has a very nice UI for iterating on graphs.

## Missing stuff before declaring 1.0

* A clear strategy for optional output streams.
  * Is the current `Option`-based solution good enough for 1.0?
* SymbolSync block at least have the right API.
* `AsRef<Path>` vs `Into<PathBuf>`?
* What exactly is the purpose of `BlockEOF`?
* Should `BufferReader` provide tags borrowed, but return owned `Vec<Tag>` from
  `consume()`? Less copying, but we need to convert to/from absolute still.
  `.tags()` could construct tags on demand for read-only view, and not require
  returning vec of tags from `read_buf()`.
* Block structs have needless trait bounds, just to be passed to generated impl
  sections.
* Should `Pending` return a time estimate?
* Or better yet: The graph should do some fancy heuristic to hone in on the
  perfect time when to call again.
  * Great for hardware like audio, SDRs.
  * Max ceiling for e.g. TCP streams.
  * Maybe both. Strobe could do with being able to just say.
* Spread the absolute stream pos type more widely?

## Contributing

Contributions are very welcome. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Browser UI

The browser components and WASM applications live in the separate
[`rustradio-ui` repository](https://github.com/ThomasHabets/rustradio-ui).
