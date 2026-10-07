#!/usr/bin/env bash
set -ueo pipefail
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.unstable"
cargo test --features unstable --lib iq_stream
cargo clippy --features unstable --lib --examples --no-deps -- -D warnings
cargo check --features wasm,unstable --target wasm32-unknown-unknown --lib
cargo tree --features wasm,unstable --target wasm32-unknown-unknown \
    --edges normal,build --invert async-channel --depth 0 --format '{f}' \
    | (if grep -Eq '(^|,)std(,|$)'; then
        echo 'ERROR: unstable enables async-channel/std for WASM' >&2
        exit 1
    fi)
cargo check -p rustradio-ui --features unstable --target wasm32-unknown-unknown
cargo tree -p rustradio-ui --features unstable --target wasm32-unknown-unknown \
    --edges normal,build --invert async-channel --depth 0 --format '{f}' \
    | (if grep -Eq '(^|,)std(,|$)'; then
        echo 'ERROR: unstable enables async-channel/std for WASM' >&2
        exit 1
    fi)
