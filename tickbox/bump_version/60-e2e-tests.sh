#!/usr/bin/env bash
exec cargo test --locked --workspace --release -- --ignored
