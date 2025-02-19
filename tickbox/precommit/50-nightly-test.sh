#!/usr/bin/env bash
set -ueo pipefail
if cargo +nightly 2> /dev/null > /dev/null && {
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.bench.nightly"
        cd "$TICKBOX_TEMPDIR/work"
        exec cargo +nightly bench --no-run -F rtlsdr
}
