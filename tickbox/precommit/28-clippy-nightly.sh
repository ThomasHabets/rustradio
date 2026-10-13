#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.clippy.nightly"
export RUSTFLAGS="${RUSTFLAGS:-} --cfg tokio_unstable"
exec cargo +nightly clippy --locked --no-deps --workspace --all-targets \
    --features "$NATIVE_FEATURES,$NIGHTLY_FEATURES" -- -D warnings
