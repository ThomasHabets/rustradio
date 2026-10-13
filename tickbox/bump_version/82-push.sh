#!/usr/bin/env bash
set -euo pipefail
# Preserve the release commit remotely if publication fails part way through.
exec git push origin HEAD:refs/heads/main
