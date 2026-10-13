#!/usr/bin/env bash
set -euo pipefail
# Cargo orders workspace uploads and waits for dependencies to become available.
exec cargo publish --workspace --locked
