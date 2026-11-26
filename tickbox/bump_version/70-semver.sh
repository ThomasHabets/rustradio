#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"

# The version bump updates the manifest before committing it.
package_version() {
    awk -F '"' '/^\[package\]$/ { package = 1; next } /^\[/ { package = 0 }
               package && /^version = "/ { print $2; exit }'
}
previous=$(git show HEAD:Cargo.toml | package_version)
release=$(package_version < Cargo.toml)
core='^([0-9]+)\.([0-9]+)\.([0-9]+)([-+].*)?$'
[[ $previous =~ $core ]] || { echo "Invalid previous version: $previous" >&2; exit 1; }
old_major=${BASH_REMATCH[1]} old_minor=${BASH_REMATCH[2]} old_patch=${BASH_REMATCH[3]}
[[ $release =~ $core ]] || { echo "Invalid release version: $release" >&2; exit 1; }
major=${BASH_REMATCH[1]} minor=${BASH_REMATCH[2]} patch=${BASH_REMATCH[3]}

if (( major > old_major ||
      (major == 0 && old_major == 0 && minor > old_minor) ||
      (major == 0 && old_major == 0 && minor == 0 && old_minor == 0 && patch > old_patch) )); then
    echo "Skipping semver check for breaking release $previous -> $release"
    exit 0
fi

exec cargo semver-checks --only-explicit-features --features "$STABLE_FEATURES"
