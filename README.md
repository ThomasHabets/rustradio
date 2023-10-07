# Rust Radio

A library for digital signals processing in the spirit of GNU Radio.

* https://github.com/ThomasHabets/rustradio
* https://crates.io/crates/rustradio

For extra speed, build with env `RUSTFLAGS="-C target-cpu=native"`

## TODO before actually usable

* Fix SymbolSync block.

## TODO before version 1.0.0

* Multiple readers from one output.
* Documentation

## Publish new version

```
./extra/bump_version.sh
```

## Benchmark

```
cargo +nightly bench
```
