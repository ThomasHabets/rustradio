#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
tickbox_target native "clippy"
exec cargo clippy --locked --no-deps --workspace --lib --bins --examples --tests --features "$NATIVE_FEATURES" -- -D warnings
