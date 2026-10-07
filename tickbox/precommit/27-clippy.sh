#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.clippy"
# The UI enables wasm on the local rustradio dependency; check it separately
# so native examples retain MTGraph.
cargo clippy --no-deps --workspace --exclude rustradio-ui -F rtlsdr,soapysdr,fast-math,audio,fftw,async,nix,pipewire -- -D warnings
# Was, and maybe should at some point be changed back to:
# exec cargo clippy --all-features --all-targets
cargo clippy -p rustradio-ui --target wasm32-unknown-unknown --no-deps -- -D warnings
