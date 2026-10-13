#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
exec cargo semver-checks --only-explicit-features --features "$STABLE_FEATURES"
