#!/usr/bin/env bash
CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | head -1 | sed 's/"//g')"
exec echo git tag "v${CURRENT}"
