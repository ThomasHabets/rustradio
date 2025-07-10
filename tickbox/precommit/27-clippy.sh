#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.clippy"
exec cargo clippy --workspace -F rtlsdr,soapysdr,fast-math,audio,fftw,async,nix
# Was, and maybe should at some point be changed back to:
# exec cargo clippy --all-features --all-targets
