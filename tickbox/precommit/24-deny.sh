#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.deny"
if [[ "${NO_NET:-}" = "true" ]]; then
        exec cargo deny --all-features --workspace --offline check
fi
exec cargo deny --all-features --workspace check
