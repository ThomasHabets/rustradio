#!/usr/bin/env bash
set -euo pipefail
source "$TICKBOX_TEMPDIR/work/tickbox/common.sh"
if [[ ${SLOW:-} = true ]]; then
    cd "$TICKBOX_TEMPDIR/work"
    IFS=, read -ra features <<< "$NATIVE_FEATURES,wasm,$NIGHTLY_FEATURES"
    for feature in "${features[@]}"; do
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.feature.${feature}"
        toolchain=stable
        flags="${RUSTFLAGS:-}"
        case "$feature" in
            simd) toolchain=nightly ;;
            tokio-unstable) toolchain=nightly; flags+=" --cfg tokio_unstable" ;;
        esac
        RUSTFLAGS="$flags" cargo "+$toolchain" test --locked --workspace \
            --no-default-features --features "$feature"
        if [[ ${CLEANUP:-} = true ]]; then
            rm -fr "${CARGO_TARGET_DIR?}"
        fi
    done
fi
