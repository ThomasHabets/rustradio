#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
tickbox_target wasm "clippy"
exec cargo clippy --locked --no-deps --lib --tests -F wasm -- -D warnings
