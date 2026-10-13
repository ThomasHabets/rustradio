#!/usr/bin/env bash
set -euo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
if [[ ${SLOW:-} = true ]]; then
    cd "$TICKBOX_TEMPDIR/work"
    tickbox_target nightly "bench.nightly"
    cargo +nightly bench --locked --no-run --features rtlsdr,fftw
    cargo +nightly bench --locked --no-run --features wasm,fftw --lib --benches
    if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
    fi
fi
