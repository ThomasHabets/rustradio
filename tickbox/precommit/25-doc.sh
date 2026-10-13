#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
tickbox_target native "doc.normal"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} --deny warnings"
exec cargo doc --locked --workspace --no-deps
