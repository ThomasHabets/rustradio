# Testing

## Current testing

* Every commit gets tested with various features and default compiler flags.
    * simd is tested with nightly.
* github workflow also runs some tests.

## Wishlist

### Test all instruction sets

Unless building on a machine with AVX2, that code doesn't get tested or even
built.

Ideally pre-commit and github would cross-compile to every arch, with every
combination of instructions available. E.g.:

```
RUSTFLAGS="-C target-feature=+avx2" cargo test
```
