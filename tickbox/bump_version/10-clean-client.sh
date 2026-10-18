#!/usr/bin/env bash
set -euo pipefail
git status
[ -z "$(git status --porcelain)" ] || {
    echo "git client not clean"
    exit 1
}
[ "$(git branch --show-current)" = main ] || {
    echo "Releases must run from main" >&2
    exit 1
}
