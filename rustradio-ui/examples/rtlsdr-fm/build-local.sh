#!/usr/bin/env bash
set -euo pipefail

WEBD="web"
PREFIX="rtlsdr-fm"
UI_MANIFEST="$(cargo metadata --format-version=1 | jq -r '.packages[] | select(.name=="rustradio-ui") | .manifest_path' | head -n1)"
UI_DIR="$(dirname "$UI_MANIFEST")"
UI_ASSETS="${UI_DIR}/assets"
UHD_IMAGES_DIR="${UHD_IMAGES_DIR:-/usr/share/uhd/images}"
UHD_FIRMWARE="${UHD_IMAGES_DIR}/usrp_b200_fw.hex"
UHD_FPGA="${UHD_IMAGES_DIR}/usrp_b200_fpga.bin"

for image in "$UHD_FIRMWARE" "$UHD_FPGA"; do
        if [[ ! -r "$image" ]]; then
                echo "Missing required USRP B200 image: $image" >&2
                echo "Install the UHD images package or set UHD_IMAGES_DIR." >&2
                exit 1
        fi
done

TMPD="$(mktemp -d)"
PROFILE="${1:-profiling}"
wasm-pack build --target web -d "$TMPD/$PREFIX" "--$PROFILE"
GIT="$(git describe --tags --dirty --always)"
cp \
        "$WEBD/index.html" \
        "$WEBD/wasm-mod.js" \
        "$TMPD/$PREFIX/"
cp "$UHD_FIRMWARE" "$UHD_FPGA" "$TMPD/$PREFIX/"
cp "$UI_ASSETS/bootstrap.js" "$TMPD/$PREFIX/rustradio-ui-bootstrap.js"
cat "$UI_ASSETS/rustradio.css" "$WEBD/style.css" > "$TMPD/$PREFIX/style.css"

sed -i "s/GIT_VERSION_NOT_SET/$GIT/g" "$TMPD/$PREFIX/index.html"
(
        cd "$TMPD" && tar czf - "$PREFIX"
) > "$PREFIX".tgz
