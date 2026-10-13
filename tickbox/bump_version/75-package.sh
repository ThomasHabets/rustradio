#!/usr/bin/env bash
set -euo pipefail
# Workspace publication verifies new interdependent versions together, without
# requiring the macro crates to have been uploaded already.
exec cargo publish --workspace --dry-run --allow-dirty --locked
