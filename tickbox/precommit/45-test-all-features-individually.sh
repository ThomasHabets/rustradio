#!/usr/bin/env bash
set -uoe pipefail
cd "$TICKBOX_TEMPDIR/work"
for feature in rtlsdr soapysdr fast-math audio fftw simd async; do
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.feature.${feature}"
        cargo +nightly test -F "${feature}"
done
