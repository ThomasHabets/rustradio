#!/usr/bin/env bash
set -euo pipefail
VERSION="$(awk '/^version/ {print $3}' Cargo.toml | head -1 | sed 's/"//g')"
# A remote release tag means all workspace crates have been published.
exec git push --atomic origin HEAD:refs/heads/main "refs/tags/v${VERSION}:refs/tags/v${VERSION}"
