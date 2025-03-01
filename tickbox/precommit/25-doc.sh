#!/usr/bin/env bash
set -ueo pipefail
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.doc.normal"
cd "$TICKBOX_TEMPDIR/work"
for dir in . rustradio_macros; do
        (
                cd "$dir"
                cargo doc --message-format=json --no-deps \
                        | jq '.message | select(.level == "warning") | .message' \
                        | (grep . && exit 1 || true)
        )
done
