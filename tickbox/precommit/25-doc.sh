#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.doc.normal"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} --deny warnings"
exec cargo doc --locked --workspace --no-deps
