#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
for dir in . rustradio_macros; do
        (
                export CARGO_TARGET_DIR="$TICKBOX_CWD/${dir}/target/${TICKBOX_BRANCH}.test.normal"
                cd "$dir"
                cargo test
        )
done
