#!/usr/bin/env bash
# TODO: Make his a feature of tickbox.
set -ueo pipefail
mkdir "$TICKBOX_TEMPDIR/work"
git archive HEAD | tar -x -C "$TICKBOX_TEMPDIR/work"
git diff --cached --binary | (
        cd "$TICKBOX_TEMPDIR/work"
        git apply --allow-empty
)
