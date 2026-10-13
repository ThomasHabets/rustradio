#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
tickbox_target native "deny"
if [[ "${NO_NET:-}" = "true" ]]; then
        exec cargo deny --locked --all-features --workspace --offline check
fi
exec cargo deny --locked --all-features --workspace check
