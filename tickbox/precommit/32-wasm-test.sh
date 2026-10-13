#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
tickbox_target wasm "test.wasm"
cargo test --locked -F wasm --lib --tests
if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
fi
