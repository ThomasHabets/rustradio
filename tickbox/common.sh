#!/usr/bin/env bash
# Shared feature groups. WASM selects a different stream backend and must be
# checked separately from hardware examples. Experimental APIs are omitted
# from semver checks, but included in native tests and lints.
STABLE_FEATURES=rtlsdr,soapysdr,fast-math,audio,fftw,async,nix,pipewire,volk,uhd
NATIVE_FEATURES="${STABLE_FEATURES},unstable"
NIGHTLY_FEATURES=simd,tokio-unstable
WASM_FEATURES=wasm,unstable
