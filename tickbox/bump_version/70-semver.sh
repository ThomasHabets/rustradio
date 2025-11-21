#!/usr/bin/env bash
exec cargo semver-checks --only-explicit-features --features rtlsdr,soapysdr,fast-math,audio,fftw,async,pipewire,volk
