#!/usr/bin/env bash
set -ueo pipefail
export RUSTFLAGS="--cfg tokio_unstable"
cargo +nightly 2> /dev/null > /dev/null && {
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.nightly.all-features"
        cd "$TICKBOX_TEMPDIR/work"
        exec cargo +nightly test --profile nodebug --all-features
}
