#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.wasm"
cargo test --locked -F wasm --lib --tests
if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
fi
