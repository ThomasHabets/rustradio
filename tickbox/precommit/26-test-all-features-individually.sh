#!/usr/bin/env bash
set -uoe pipefail
if [[ ${FAST:-} = "true" ]]; then
        exit 0
fi
cd "$TICKBOX_TEMPDIR/work"
for feature in rtlsdr soapysdr fast-math audio fftw simd async nix pipewire; do
        export CARGO_TARGET_DIR="$TICKBOX_CWD/target/${TICKBOX_BRANCH}.test.feature.${feature}"
        cargo +nightly test -F "${feature}"
done
