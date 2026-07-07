#!/usr/bin/env bash
set -euo pipefail

WEBD="web"
PREFIX="rtlsdr-fm"
UI_MANIFEST="$(cargo metadata --format-version=1 | jq -r '.packages[] | select(.name=="rustradio-ui") | .manifest_path' | head -n1)"
UI_DIR="$(dirname "$UI_MANIFEST")"
UI_ASSETS="${UI_DIR}/assets"

TMPD="$(mktemp -d)"
PROFILE="${1:-release}"
wasm-pack build --target web -d "$TMPD/$PREFIX" "--$PROFILE"
GIT="$(git describe --tags --dirty --always)"
cp \
        "$WEBD/index.html" \
        "$WEBD/wasm-mod.js" \
        "$TMPD/$PREFIX/"
cp "$UI_ASSETS/bootstrap.js" "$TMPD/$PREFIX/rustradio-ui-bootstrap.js"
cat "$UI_ASSETS/rustradio.css" > "$TMPD/$PREFIX/style.css"

sed -i "s/GIT_VERSION_NOT_SET/$GIT/g" "$TMPD/$PREFIX/index.html"
(
        cd "$TMPD" && tar czf - "$PREFIX"
) > "$PREFIX".tgz
