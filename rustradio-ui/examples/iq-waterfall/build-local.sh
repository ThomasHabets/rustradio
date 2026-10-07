#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
wasm-pack build \
        --target web \
        --out-dir web-dist \
        "--${1:-dev}" \
        --features unstable
cp web/index.html web/wasm-mod.js web-dist/
cp ../../assets/bootstrap.js web-dist/rustradio-ui-bootstrap.js
cat ../../assets/rustradio.css web/style.css > web-dist/style.css
