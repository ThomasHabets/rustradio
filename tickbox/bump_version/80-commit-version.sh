#!/usr/bin/env bash
set -euo pipefail
CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | head -1 | sed 's/"//g')"
exec echo git commit -a -m"Bump version to ${VERSION}"
