#!/usr/bin/env bash
set -euo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
if [[ ${SLOW:-} = true ]]; then
    cd "$TICKBOX_TEMPDIR/work"
    export RUSTFLAGS="${RUSTFLAGS:-} --cfg tokio_unstable"
    export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.nightly.all-features"
    cargo +nightly test --locked --workspace --features "$NATIVE_FEATURES,$NIGHTLY_FEATURES"
    if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
    fi
fi
