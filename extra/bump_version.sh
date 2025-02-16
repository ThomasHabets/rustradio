#!/usr/bin/env bash
#
# Automate bumping version and publishing.
#
# Running without args bumps the patch level version.
# Alternatively, provide the new version as the first arg.
#
set -ueo pipefail

# Check that client is clean.
[ -z "$(git status --porcelain)" ] || {
    echo "git client not clean"
    exit 1
}

# Provide the new version.
CURRENT="$(awk '/^version/ {print $3}' Cargo.toml | head -1 | sed 's/"//g')"
AUTO_NEW_VERSION="$(echo $CURRENT | awk -F. '{print $1 "." $2 "." $3+1}')"
NEW="${1:-$AUTO_NEW_VERSION}"
echo "Current: '$CURRENT', New: '$NEW'"
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" Cargo.toml
sed -i -r 's/^(rustradio_macros.*version = ")[0-9.]+(".*)/\1'"${NEW}"'\2/' Cargo.toml
sed -i "s/^version = \"${CURRENT?}\"/version = \"${NEW?}\"/" rustradio_macros/Cargo.toml

# At least one of these should update Cargo.locks, I hope.
# This of course in addition to running the tests one more time.
cargo build
cargo test
(
        cd rustradio_macros
        cargo build
        cargo test
)

echo "=== Run E2E tests ==="
cargo test -- --ignored

echo "=== Run semver checks ==="
cargo semver-checks

echo "=== Commit, tag, and push ==="
git commit -a -m"Bump version to ${NEW?}"
git tag "v${NEW?}"
git push
git push --tags
(cd rustradio_macros/ && cargo publish)
cargo publish
