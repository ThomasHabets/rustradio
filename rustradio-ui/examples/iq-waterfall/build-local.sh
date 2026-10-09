#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
# Use one source version for compilation and HTML, including cached builds.
export IQ_WATERFALL_GIT_VERSION="$(git describe --tags --dirty --always)"
wasm-pack build \
        --target web \
        --out-dir web-dist \
        "--${1:-dev}" \
        --features unstable
cp web/index.html web/wasm-mod.js web-dist/
python3 - <<'PY'
import html
import os
from pathlib import Path

path = Path("web-dist/index.html")
path.write_text(path.read_text().replace(
    "GIT_VERSION_NOT_SET", html.escape(os.environ["IQ_WATERFALL_GIT_VERSION"], quote=True)
))
PY
cp ../../assets/bootstrap.js web-dist/rustradio-ui-bootstrap.js
cat ../../assets/rustradio.css web/style.css > web-dist/style.css
