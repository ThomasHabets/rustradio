#!/usr/bin/env bash
set -ueo pipefail

CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | sed 's/"//g')"
NEW="$(echo $CURRENT | awk -F. '{print $1 "." $2 "." $3+1}')"
echo "Current: '$CURRENT', New: '$NEW'"
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" Cargo.toml
exec cargo build
