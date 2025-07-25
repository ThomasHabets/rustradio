#!/usr/bin/env bash
set -ueo pipefail
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.all-features"
cd "$TICKBOX_TEMPDIR/work"
exec cargo test --workspace --features rtlsdr,soapysdr,fast-math,audio,fftw,async,pipewire
