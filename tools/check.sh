#!/usr/bin/env sh
# Run before every commit: fails (non-zero) on any clippy warning or test failure.
set -e
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
