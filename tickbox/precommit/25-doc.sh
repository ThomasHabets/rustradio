#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.doc.normal"
cargo doc --message-format=json --no-deps \
        | jq '.message | select(.level == "warning") | .message' \
        | (grep -q . && {
                echo "There are warnings related to docs:"
                cargo doc --no-deps
                exit 1
        } || true)
