# Rust Radio

A library for digital signals processing in the spirit of GNU Radio.

* https://github.com/ThomasHabets/rustradio
* https://crates.io/crates/rustradio

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
* Example AX.25 KISS modem written, using soapysdr.
* `AsRef<Path>` vs `Into<PathBuf>`?
* What exactly is the purpose of `BlockEOF`?
* Should `produce()` take `Into<Vec<Tag>>`? Less copying.
