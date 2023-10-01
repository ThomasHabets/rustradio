#!/usr/bin/env bash
set -ueo pipefail

# Check that client is clean.
[ -z "$(git status --porcelain)" ] || {
    echo "git client not clean"
    exit 1
}

CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | sed 's/"//g')"
NEW="$(echo $CURRENT | awk -F. '{print $1 "." $2 "." $3+1}')"
echo "Current: '$CURRENT', New: '$NEW'"
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" Cargo.toml
cargo build
echo git commit -a -m"Bump version to ${NEW?}"
