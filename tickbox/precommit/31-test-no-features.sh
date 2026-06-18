#!/usr/bin/env bash
set -ueo pipefail
if [[ ${SLOW:-} = "true" ]]; then
        cd "$TICKBOX_TEMPDIR/work"
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.no-features"
        cargo test --workspace --no-default-features
        if [[ ${CLEANUP:-} = true ]]; then
                rm -fr "${CARGO_TARGET_DIR?}"
        fi
fi
