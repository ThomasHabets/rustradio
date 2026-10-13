#!/usr/bin/env bash
set -euo pipefail
if [[ ${SLOW:-} = true ]]; then
    cd "$TICKBOX_TEMPDIR/work"
    export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.bench.nightly"
    cargo +nightly bench --locked --no-run --features rtlsdr,fftw
    cargo +nightly bench --locked --no-run --features wasm,fftw --lib --benches
    if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
    fi
fi
