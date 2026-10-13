#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
if [[ ${SLOW:-} = "true" ]]; then
        cd "$TICKBOX_TEMPDIR/work"
        tickbox_target native "test.normal"
        cargo test --locked --workspace
        if [[ ${CLEANUP:-} = true ]]; then
                rm -fr "${CARGO_TARGET_DIR?}"
        fi
fi
