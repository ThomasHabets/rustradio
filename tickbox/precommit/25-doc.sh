#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
for dir in . rustradio_macros; do
        export CARGO_TARGET_DIR="$TICKBOX_CWD/${dir}/target/${TICKBOX_BRANCH}.doc.normal"
        (
                cd "$dir"
                cargo doc --message-format=json --no-deps \
                        | jq '.message | select(.level == "warning") | .message' \
                        | (grep -q . && {
                                echo "There are warnings related to docs:"
                                cargo doc --no-deps
                                exit 1
                        } || true)
        )
done
