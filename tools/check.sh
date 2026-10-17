#!/usr/bin/env sh
# Run before every commit: fails (non-zero) on any type error, clippy warning or test failure.
# The Tauri app embeds the built front end, so it is type-checked and built first.
set -e
cd "$(dirname "$0")/.."
(
  cd app
  [ -d node_modules ] || npm ci --no-audit --no-fund
  npm run --silent check
  npm run --silent build >/dev/null
)
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
