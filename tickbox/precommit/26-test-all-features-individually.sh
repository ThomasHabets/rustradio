#!/usr/bin/env bash
set -euo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
if [[ ${SLOW:-} = true ]]; then
    cd "$TICKBOX_TEMPDIR/work"
    IFS=, read -ra features <<< "$NATIVE_FEATURES,wasm,$NIGHTLY_FEATURES"
    for feature in "${features[@]}"; do
        echo "Testing feature $feature"
        toolchain=stable
        flags="${RUSTFLAGS:-}"
        case "$feature" in
            simd) toolchain=nightly ;;
            tokio-unstable) flags+=" --cfg tokio_unstable" ;;
        esac
        backend=native
        [[ "$toolchain" = nightly ]] && backend=nightly
        [[ "$feature" = wasm ]] && backend=wasm
        tickbox_target "$backend" "test.feature.${feature}"
        if [[ ${feature} = wasm ]]; then
            # Don't try to build examples.
            RUSTFLAGS="$flags" cargo "+$toolchain" test --locked --workspace \
                --no-default-features --features wasm --lib
        else
            RUSTFLAGS="$flags" cargo "+$toolchain" test --locked --workspace \
                --no-default-features --features "$feature"
        fi
        if [[ ${CLEANUP:-} = true ]]; then
            rm -fr "${CARGO_TARGET_DIR?}"
        fi
    done
fi
