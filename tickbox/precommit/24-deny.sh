#!/usr/bin/env bash
set -ueo pipefail
if [[ "${NO_NET:-}" = "true" ]]; then
        exit 0
fi
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.deny"
exec cargo deny --all-features --workspace check
