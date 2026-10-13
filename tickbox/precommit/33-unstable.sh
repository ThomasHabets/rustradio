#!/usr/bin/env bash
set -ueo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
cd "$TICKBOX_TEMPDIR/work"
export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.unstable"
cargo test --locked --features unstable --lib --tests
cargo clippy --locked --features unstable --lib --bins --examples --tests --no-deps -- -D warnings
cargo check --locked --features "$WASM_FEATURES" --target wasm32-unknown-unknown --lib
cargo tree --locked --features "$WASM_FEATURES" --target wasm32-unknown-unknown \
    --edges normal,build --invert async-channel --depth 0 --format '{f}' \
    | (if grep -Eq '(^|,)std(,|$)'; then
        echo 'ERROR: unstable enables async-channel/std for WASM' >&2
        exit 1
    fi)
