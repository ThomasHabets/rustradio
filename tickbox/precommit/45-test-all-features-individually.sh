#!/usr/bin/env bash
set -uoe pipefail
for feature in rtlsdr soapysdr fast-math audio fftw simd; do
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.feature.${feature}"
        cargo +nightly test --profile=nodebug -F "${feature}"
done
