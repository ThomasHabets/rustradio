#!/usr/bin/env bash
set -ueo pipefail

# Check that client is clean.
[ -z "$(git status --porcelain)" ] || {
    echo "git client not clean"
    exit 1
}

CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | sed 's/"//g')"
if [[ "${1:-}" = "" ]]; then
    NEW="$(echo $CURRENT | awk -F. '{print $1 "." $2 "." $3+1}')"
else
    NEW="$1"
fi
echo "Current: '$CURRENT', New: '$NEW'"
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" Cargo.toml
sed -i -r 's/^(rustradio_macros.*version = ")[0-9.]+(".*)/\1'"${NEW}"'\2/' Cargo.toml
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" rustradio_macros/Cargo.toml

# At least one of these should update Cargo.locks, I hope.
cargo build
cargo test

git commit -a -m"Bump version to ${NEW?}"
cargo semver-checks
git tag "v${NEW?}"
git push
git push --tags
(cd rustradio_macros/ && cargo publish) && cargo publish
