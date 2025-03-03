#!/usr/bin/env bash
set -euo pipefail
CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | head -1 | sed 's/"//g')"
AUTO_NEW_VERSION="$(echo $CURRENT | awk -F. '{print $1 "." $2 "." $3+1}')"
NEW="${1:-$AUTO_NEW_VERSION}"

# MANUAL_VERSION is set manually by the person running the release.
NEW="${MANUAL_VERSION:-$NEW}"
echo "Current: '$CURRENT', New: '$NEW'"
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" Cargo.toml
sed -i -r 's/^(rustradio_macros.*version = ")[0-9.]+(".*)/\1'"${NEW}"'\2/' Cargo.toml
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" rustradio_macros/Cargo.toml
