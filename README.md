# Rust Radio

A library for digital signals processing in the spirit of GNU Radio.

* https://github.com/ThomasHabets/rustradio
* https://crates.io/crates/rustradio

For extra speed(?), build with env `RUSTFLAGS="-C target-cpu=native"`

## Publish new version

```
./extra/bump_version.sh
git push && cargo publish
```

## Benchmark

```
cargo +nightly bench
```
