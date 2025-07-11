#!/usr/bin/env bash
set -euo pipefail
for dir in rustradio_macros_code rustradio_macros; do
        cargo publish
done
