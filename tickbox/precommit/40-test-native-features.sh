#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
tickbox_target native "test.all-features"
cd "$TICKBOX_TEMPDIR/work"
cargo test --locked --workspace --features "$NATIVE_FEATURES"
if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
fi
