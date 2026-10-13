#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.all-features"
cd "$TICKBOX_TEMPDIR/work"
cargo test --locked --workspace --features "$NATIVE_FEATURES"
if [[ ${CLEANUP:-} = true ]]; then
        rm -fr "${CARGO_TARGET_DIR?}"
fi
