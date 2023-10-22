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

## Useful commands

Plot I/Q data

```
$ od -A none -w8 -f test.c32 > t
$ gnuplot
gnuplot> plot 't' using 1 w l, 't' using 2 w l
```
