#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.clippy.nightly"
export RUSTFLAGS="--cfg tokio_unstable"
exec cargo +nightly clippy --workspace --all-targets -F simd,rtlsdr,soapysdr,fast-math,audio,fftw,async,tokio-unstable,nix,pipewire
