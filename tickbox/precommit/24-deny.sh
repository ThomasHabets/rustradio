#!/usr/bin/env bash
set -ueo pipefail
if [[ "${NO_NET:-}" = "true" ]]; then
        exit 0
fi
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.deny"
exec cargo deny check
# Was, and maybe should at some point be changed back to:
# exec cargo clippy --all-features --all-targets
