#!/usr/bin/env bash
set -ueo pipefail
export RUSTFLAGS="--cfg tokio_unstable"
cargo +nightly 2> /dev/null > /dev/null && {
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.nightly.all-features"
        cd "$TICKBOX_TEMPDIR/work"
        # This is not "all features" because wasm.
        exec cargo +nightly test --workspace -F simd,rtlsdr,soapysdr,fast-math,audio,fftw,async,tokio-unstable,nix,pipewire
}
