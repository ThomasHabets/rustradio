#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
for dir in . rustradio_macros; do
        (
                cd "$dir"
                cargo fmt -- --check
        )
done
