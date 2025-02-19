#!/usr/bin/env bash
set -ueo pipefail
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.normal"
cd "$TICKBOX_TEMPDIR/work"
for dir in . rustradio_macros; do
        (
                cd "$dir"
                cargo test
        )
done
